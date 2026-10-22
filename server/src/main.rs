//! post.blinker.cool: takes files from blinker, keeps them for a few days and
//! shows each on a page of its own: an image viewer, a video or audio player
//! with the markers set in blinker's video editor.
//!
//! The upload protocol is the app's (`src/upload.rs`):
//!
//! ```text
//! POST /api/upload
//! Authorization: Bearer <token>          (when TOKEN is set)
//! Content-Type: image/png | … | video/mp4 | audio/mpeg
//! X-Filename: <percent-encoded name>
//! X-Blinker-Markers: <percent-encoded [{"t": 1.5, "text": "…"}]>
//! <the file>
//!
//! 201 {"url": "https://…/<id>", "raw_url": "…", "delete_url": "…", "expires": <unix>}
//! ```
//!
//! Settings come from the environment (see `Settings::from_env`); files and
//! their notes live in `DATA_DIR`, one `<id>.<ext>` and one `<id>.json` each.

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

const PAGE: &str = include_str!("page.html");
const HOME: &str = include_str!("home.html");

struct Settings {
    bind: String,
    data: PathBuf,
    /// Links are made with this, e.g. https://post.blinker.cool
    public_url: String,
    ttl: u64,
    /// Files bigger than `big_bytes` live `big_ttl` instead.
    big_bytes: u64,
    big_ttl: u64,
    max_bytes: u64,
    /// The data folder never grows past this.
    disk_limit: u64,
    /// Uploads per address per hour.
    per_hour: usize,
    /// Empty = anyone may upload.
    token: String,
    /// Behind a proxy that sets X-Real-IP (nginx, caddy).
    trust_proxy: bool,
}

impl Settings {
    fn from_env() -> Settings {
        let var = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| d.to_string());
        let num = |k: &str, d: u64| var(k, &d.to_string()).trim().parse().unwrap_or(d);
        Settings {
            bind: var("BIND", "0.0.0.0:8080"),
            data: PathBuf::from(var("DATA_DIR", "./data")),
            public_url: var("PUBLIC_URL", "http://localhost:8080").trim_end_matches('/').to_string(),
            ttl: num("TTL_HOURS", 72) * 3600,
            big_bytes: num("BIG_MB", 25) * 1024 * 1024,
            big_ttl: num("BIG_TTL_HOURS", 24) * 3600,
            max_bytes: num("MAX_MB", 100) * 1024 * 1024,
            disk_limit: num("DISK_LIMIT_GB", 20) * 1024 * 1024 * 1024,
            per_hour: num("UPLOADS_PER_HOUR", 120) as usize,
            token: var("TOKEN", ""),
            trust_proxy: var("TRUST_PROXY", "0") == "1",
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Marker {
    t: f64,
    #[serde(default)]
    text: String,
}

/// What we know about a stored file (`<id>.json`).
#[derive(Serialize, Deserialize, Clone)]
struct Meta {
    id: String,
    name: String,
    /// "image", "video" or "audio"
    kind: String,
    mime: String,
    ext: String,
    size: u64,
    created: u64,
    expires: u64,
    #[serde(default)]
    markers: Vec<Marker>,
    delete_token: String,
    /// Pixels of an image (for the previews in messengers).
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

struct App {
    s: Settings,
    /// Bytes in the data folder.
    used: AtomicU64,
    /// Recent upload times per address.
    recent: Mutex<HashMap<String, Vec<u64>>>,
    /// At most a few uploads are held in memory at once.
    slots: tokio::sync::Semaphore,
    /// One eviction at a time.
    evicting: tokio::sync::Mutex<()>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_id(n: usize) -> String {
    const ABC: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("random bytes");
    bytes.iter().map(|b| ABC[*b as usize % ABC.len()] as char).collect()
}

fn valid_id(id: &str) -> bool {
    (6..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The file type from its first bytes (what the client says is not trusted):
/// (kind, mime, extension).
fn sniff(b: &[u8]) -> Option<(&'static str, &'static str, &'static str)> {
    let at = |i: usize, s: &[u8]| b.len() >= i + s.len() && &b[i..i + s.len()] == s;
    Some(if at(0, b"\x89PNG") {
        ("image", "image/png", "png")
    } else if at(0, b"\xFF\xD8\xFF") {
        ("image", "image/jpeg", "jpg")
    } else if at(0, b"GIF8") {
        ("image", "image/gif", "gif")
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        ("image", "image/webp", "webp")
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        ("audio", "audio/wav", "wav")
    } else if at(4, b"ftyp") {
        // M4A / MP4: the brand tells sound from video
        if at(8, b"M4A ") { ("audio", "audio/mp4", "m4a") } else { ("video", "video/mp4", "mp4") }
    } else if at(0, b"ID3") || (b.len() > 2 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0) {
        ("audio", "audio/mpeg", "mp3")
    } else {
        return None;
    })
}

/// Width and height of a PNG, GIF, JPEG or WebP from its header.
fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let le16 = |i: usize| Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]));
    let le24 = |i: usize| Some(*b.get(i)? as u32 | (*b.get(i + 1)? as u32) << 8 | (*b.get(i + 2)? as u32) << 16);
    if b.starts_with(b"\x89PNG") {
        return Some((be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if b.len() > 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return match &b[12..16] {
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            b"VP8L" => {
                let v = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
                Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    if b.starts_with(b"\xFF\xD8") {
        // walk the segments to the frame header (SOF0..SOF15 but DHT, JPG, DAC)
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                return None;
            }
            let m = b[i + 1];
            let len = be16(i + 2)? as usize;
            if (0xC0..=0xCF).contains(&m) && ![0xC4, 0xC8, 0xCC].contains(&m) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + len;
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A file name to show: no folders, no control characters, not too long.
fn clean_name(raw: &str, ext: &str) -> String {
    let name: String = raw.rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|c| !c.is_control()).take(120).collect();
    let name = name.trim();
    if name.is_empty() { format!("blinker.{ext}") } else { name.to_string() }
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn client_ip(app: &App, headers: &HeaderMap, addr: SocketAddr) -> String {
    if app.s.trust_proxy
        && let Some(ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok())
    {
        return ip.trim().to_string();
    }
    addr.ip().to_string()
}

async fn upload(State(app): State<Arc<App>>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap, body: Bytes) -> Response {
    if !app.s.token.is_empty() {
        let ok = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.strip_prefix("Bearer ").is_some_and(|t| t.trim() == app.s.token));
        if !ok {
            return error(StatusCode::UNAUTHORIZED, "wrong token");
        }
    }
    let _slot = app.slots.acquire().await;
    if body.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty file");
    }
    if body.len() as u64 > app.s.max_bytes {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "file too large");
    }
    let ip = client_ip(&app, &headers, addr);
    {
        let t = now();
        let mut recent = app.recent.lock().unwrap();
        let list = recent.entry(ip.clone()).or_default();
        list.retain(|x| t - *x < 3600);
        if list.len() >= app.s.per_hour {
            return error(StatusCode::TOO_MANY_REQUESTS, "too many uploads, try again in an hour");
        }
        list.push(t);
    }
    if app.used.load(Ordering::Relaxed) + body.len() as u64 > app.s.disk_limit && !make_room(&app, body.len() as u64).await {
        return error(StatusCode::INSUFFICIENT_STORAGE, "the server is full, try again later");
    }
    let Some((kind, mime, ext)) = sniff(&body) else {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "only images, mp4 videos and sound");
    };
    let raw_name = headers.get("x-filename").and_then(|v| v.to_str().ok()).map(percent_decode).unwrap_or_default();
    let mut markers: Vec<Marker> = headers
        .get("x-blinker-markers")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| serde_json::from_str(&percent_decode(v)).ok())
        .unwrap_or_default();
    markers.retain(|m| m.t.is_finite() && m.t >= 0.0);
    markers.truncate(200);
    for m in &mut markers {
        m.text = m.text.chars().filter(|c| !c.is_control() || *c == '\n').take(300).collect();
    }

    let id = random_id(10);
    let created = now();
    let (width, height) = if kind == "image" { image_size(&body).unwrap_or((0, 0)) } else { (0, 0) };
    let meta = Meta {
        id: id.clone(),
        name: clean_name(&raw_name, ext),
        kind: kind.into(),
        mime: mime.into(),
        ext: ext.into(),
        size: body.len() as u64,
        created,
        expires: created + if body.len() as u64 > app.s.big_bytes { app.s.big_ttl } else { app.s.ttl },
        markers,
        delete_token: random_id(24),
        width,
        height,
    };
    let file = app.s.data.join(format!("{id}.{ext}"));
    if let Err(e) = tokio::fs::write(&file, &body).await {
        eprintln!("write {}: {e}", file.display());
        return error(StatusCode::INTERNAL_SERVER_ERROR, "could not store the file");
    }
    let json = serde_json::to_vec_pretty(&meta).unwrap_or_default();
    if let Err(e) = tokio::fs::write(app.s.data.join(format!("{id}.json")), json).await {
        eprintln!("write meta {id}: {e}");
        let _ = tokio::fs::remove_file(&file).await;
        return error(StatusCode::INTERNAL_SERVER_ERROR, "could not store the file");
    }
    app.used.fetch_add(meta.size, Ordering::Relaxed);
    println!("upload {id} {kind} {} bytes from {ip}", meta.size);
    let base = &app.s.public_url;
    let answer = serde_json::json!({
        "url": format!("{base}/{id}"),
        "raw_url": format!("{base}/f/{id}.{ext}"),
        "delete_url": format!("{base}/api/delete/{id}?token={}", meta.delete_token),
        "expires": meta.expires,
    });
    (StatusCode::CREATED, Json(answer)).into_response()
}

async fn read_meta(app: &App, id: &str) -> Option<Meta> {
    if !valid_id(id) {
        return None;
    }
    let text = tokio::fs::read(app.s.data.join(format!("{id}.json"))).await.ok()?;
    let meta: Meta = serde_json::from_slice(&text).ok()?;
    (meta.expires > now()).then_some(meta)
}

/// `<` and `&` escaped for HTML text and attributes.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

async fn page(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let Some(meta) = read_meta(&app, &id).await else {
        return gone();
    };
    let base = &app.s.public_url;
    let raw = format!("{base}/f/{}.{}", meta.id, meta.ext);
    let og = match meta.kind.as_str() {
        // the picture's size: with it Telegram and the like show it whole
        // and large instead of a small square
        "image" => {
            let size = if meta.width > 0 && meta.height > 0 {
                format!(r#"<meta property="og:image:width" content="{}"><meta property="og:image:height" content="{}">"#, meta.width, meta.height)
            } else {
                String::new()
            };
            format!(
                r#"<meta property="og:type" content="website"><meta property="og:image" content="{raw}"><meta property="og:image:secure_url" content="{raw}"><meta property="og:image:type" content="{}">{size}<meta name="twitter:card" content="summary_large_image"><meta name="twitter:image" content="{raw}">"#,
                meta.mime
            )
        }
        "video" => format!(
            r#"<meta property="og:type" content="video.other"><meta property="og:video" content="{raw}"><meta property="og:video:secure_url" content="{raw}"><meta property="og:video:type" content="video/mp4"><meta name="twitter:card" content="player">"#
        ),
        _ => format!(r#"<meta property="og:audio" content="{raw}"><meta property="og:audio:type" content="{}">"#, meta.mime),
    };
    let data = serde_json::json!({
        "id": meta.id,
        "name": meta.name,
        "kind": meta.kind,
        "mime": meta.mime,
        "size": meta.size,
        "created": meta.created,
        "expires": meta.expires,
        "raw": raw,
        "url": format!("{base}/{}", meta.id),
        "markers": meta.markers,
    });
    // JSON inside <script>: no way out of it
    let data = data.to_string().replace("</", "<\\/");
    let html = PAGE
        .replace("{{TITLE}}", &esc(&meta.name))
        .replace("{{OG}}", &og)
        .replace("{{URL}}", &esc(&format!("{base}/{}", meta.id)))
        .replace("{{DATA}}", &data);
    let mut r = Html(html).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

fn gone() -> Response {
    let html = PAGE
        .replace("{{TITLE}}", "blinker")
        .replace("{{OG}}", "")
        .replace("{{URL}}", "")
        .replace("{{DATA}}", "null");
    (StatusCode::NOT_FOUND, Html(html)).into_response()
}

async fn file(State(app): State<Arc<App>>, Path(name): Path<String>, req: Request) -> Response {
    let Some((id, ext)) = name.split_once('.') else { return StatusCode::NOT_FOUND.into_response() };
    let Some(meta) = read_meta(&app, id).await else { return StatusCode::NOT_FOUND.into_response() };
    if meta.ext != ext {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = app.s.data.join(format!("{}.{}", meta.id, meta.ext));
    let mut r = match tower_http::services::ServeFile::new_with_mime(&path, &meta.mime.parse().unwrap_or(mime_octet())).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let h = r.headers_mut();
    let left = meta.expires.saturating_sub(now());
    // browsers keep it until it expires; a CDN only for a while, so a
    // deleted file goes away soon
    if let Ok(v) = HeaderValue::from_str(&format!("public, max-age={left}, s-maxage=600")) {
        h.insert(header::CACHE_CONTROL, v);
    }
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; media-src 'self'; img-src 'self'"));
    let ascii: String = meta.name.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '_' }).filter(|c| *c != '"').collect();
    let utf8: String = meta.name.bytes().map(|b| if b.is_ascii_alphanumeric() || b".-_".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{ascii}\"; filename*=UTF-8''{utf8}")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

fn mime_octet() -> mime::Mime {
    mime::APPLICATION_OCTET_STREAM
}

#[derive(Deserialize)]
struct DeleteQuery {
    token: String,
}

async fn delete(State(app): State<Arc<App>>, Path(id): Path<String>, Query(q): Query<DeleteQuery>) -> Response {
    let Some(meta) = read_meta(&app, &id).await else { return error(StatusCode::NOT_FOUND, "no such file") };
    if meta.delete_token != q.token {
        return error(StatusCode::FORBIDDEN, "wrong token");
    }
    remove(&app, &meta).await;
    (StatusCode::OK, Html("<!doctype html><meta charset=utf-8><title>blinker</title><body style=\"background:#161615;color:#e6e4de;font:16px system-ui;display:grid;place-items:center;height:100vh;margin:0\">deleted · удалено</body>")).into_response()
}

async fn remove(app: &App, meta: &Meta) {
    let _ = tokio::fs::remove_file(app.s.data.join(format!("{}.{}", meta.id, meta.ext))).await;
    let _ = tokio::fs::remove_file(app.s.data.join(format!("{}.json", meta.id))).await;
    app.used.fetch_sub(meta.size.min(app.used.load(Ordering::Relaxed)), Ordering::Relaxed);
}

/// The disk is full: the oldest files go first until `need` bytes fit.
/// False if even an empty folder couldn't take it.
async fn make_room(app: &App, need: u64) -> bool {
    let _lock = app.evicting.lock().await;
    let mut used = sweep(app).await;
    if used + need <= app.s.disk_limit {
        app.used.store(used, Ordering::Relaxed);
        return true;
    }
    let mut metas = all_meta(app).await;
    metas.sort_by_key(|m| m.created);
    for m in metas {
        if used + need <= app.s.disk_limit {
            break;
        }
        println!("evicted {} ({} bytes) to make room", m.id, m.size);
        remove(app, &m).await;
        used = used.saturating_sub(m.size);
    }
    app.used.store(used, Ordering::Relaxed);
    used + need <= app.s.disk_limit
}

/// Notes of every stored file.
async fn all_meta(app: &App) -> Vec<Meta> {
    let mut out = Vec::new();
    let Ok(mut dir) = tokio::fs::read_dir(&app.s.data).await else { return out };
    while let Ok(Some(e)) = dir.next_entry().await {
        if e.file_name().to_string_lossy().ends_with(".json")
            && let Some(m) = tokio::fs::read(e.path()).await.ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok())
        {
            out.push(m);
        }
    }
    out
}

/// Deletes expired files (and strays without notes); returns the bytes kept.
async fn sweep(app: &App) -> u64 {
    let mut kept = 0;
    let Ok(mut dir) = tokio::fs::read_dir(&app.s.data).await else { return 0 };
    let t = now();
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    while let Ok(Some(e)) = dir.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".json") {
            match tokio::fs::read(e.path()).await.ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok()) {
                Some(m) if m.expires > t => {
                    kept += m.size;
                    seen.insert(m.id.clone());
                }
                Some(m) => {
                    println!("expired {}", m.id);
                    let _ = tokio::fs::remove_file(app.s.data.join(format!("{}.{}", m.id, m.ext))).await;
                    let _ = tokio::fs::remove_file(e.path()).await;
                }
                None => {
                    let _ = tokio::fs::remove_file(e.path()).await;
                    let _ = id;
                }
            }
        } else {
            files.push((name, e.path()));
        }
    }
    // a file whose note is gone (an upload cut short)
    for (name, path) in files {
        let id = name.split('.').next().unwrap_or("");
        let old = tokio::fs::metadata(&path).await.ok().and_then(|m| m.modified().ok()).and_then(|m| m.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(600));
        if !seen.contains(id) && old {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
    kept
}

async fn home() -> Html<&'static str> {
    Html(HOME)
}

async fn asset(Path(name): Path<String>) -> Response {
    let (bytes, mime): (&'static [u8], &str) = match name.as_str() {
        "FiraSans-Regular.ttf" => (include_bytes!("../../assets/fonts/FiraSans-Regular.ttf"), "font/ttf"),
        "FiraSans-Medium.ttf" => (include_bytes!("../../assets/fonts/FiraSans-Medium.ttf"), "font/ttf"),
        "FiraSans-SemiBold.ttf" => (include_bytes!("../../assets/fonts/FiraSans-SemiBold.ttf"), "font/ttf"),
        "FiraMono-Regular.ttf" => (include_bytes!("../../assets/fonts/FiraMono-Regular.ttf"), "font/ttf"),
        "icon.svg" => (include_bytes!("icon.svg"), "image/svg+xml"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=604800")], bytes).into_response()
}

#[tokio::main]
async fn main() {
    let s = Settings::from_env();
    if let Err(e) = std::fs::create_dir_all(&s.data) {
        eprintln!("data folder {}: {e}", s.data.display());
        std::process::exit(1);
    }
    let max = s.max_bytes as usize;
    let bind = s.bind.clone();
    let app = Arc::new(App {
        s,
        used: AtomicU64::new(0),
        recent: Mutex::new(HashMap::new()),
        slots: tokio::sync::Semaphore::new(4),
        evicting: tokio::sync::Mutex::new(()),
    });
    app.used.store(sweep(&app).await, Ordering::Relaxed);
    let a2 = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            a2.used.store(sweep(&a2).await, Ordering::Relaxed);
            let t = now();
            a2.recent.lock().unwrap().retain(|_, v| v.iter().any(|x| t - *x < 3600));
        }
    });
    let router = Router::new()
        .route("/", get(home))
        .route("/health", get(|| async { "ok" }))
        .route("/api/upload", post(upload).layer(DefaultBodyLimit::max(max + 1024 * 1024)))
        .route("/api/delete/{id}", get(delete))
        .route("/assets/{name}", get(asset))
        .route("/f/{name}", get(file))
        .route("/{id}", get(page))
        .with_state(app.clone());
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    println!("blinker-post on {bind}, {} kept, links {}", app.used.load(Ordering::Relaxed), app.s.public_url);
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let shutdown = async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal");
        tokio::select! {
            _ = shutdown => {}
            _ = term.recv() => {}
        }
    };
    let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types() {
        assert_eq!(sniff(b"\x89PNG\r\n").map(|t| t.2), Some("png"));
        assert_eq!(sniff(b"\0\0\0 ftypisom").map(|t| t.2), Some("mp4"));
        assert_eq!(sniff(b"\0\0\0 ftypM4A ").map(|t| t.2), Some("m4a"));
        assert_eq!(sniff(b"ID3\x04").map(|t| t.2), Some("mp3"));
        assert_eq!(sniff(b"<html>"), None);
    }

    #[test]
    fn sizes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&840u32.to_be_bytes());
        png.extend_from_slice(&636u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((840, 636)));
        assert_eq!(image_size(b"GIF89a\x20\x03\x58\x02"), Some((800, 600)));
        assert_eq!(image_size(b"<html>"), None);
    }

    #[test]
    fn names() {
        assert_eq!(percent_decode("blinker%20%D0%BF.png"), "blinker п.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(clean_name("../../etc/passwd", "png"), "passwd");
        assert_eq!(clean_name("", "png"), "blinker.png");
    }
}
