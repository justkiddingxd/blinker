//! Upload to your own server and put the link on the clipboard.
//!
//! Protocol (the server side implements this):
//!
//! ```text
//! POST <upload_url>
//! Authorization: Bearer <upload_token>        (omitted when the token is empty)
//! Content-Type: image/png | image/jpeg | image/webp | image/gif | video/mp4
//! Content-Length: <size>
//! X-Filename: <file name, percent-encoded UTF-8>
//! X-Blinker-Markers: <percent-encoded JSON>   (videos with timeline markers)
//!     [{"t": 12.5, "text": "what happens here"}, ...]   t in seconds
//! <raw file bytes>
//!
//! 200 / 201  {"url": "https://…", "delete_url": "https://…" (optional)}
//! 4xx / 5xx  {"error": "message"}  (or any text)
//! ```
//!
//! With no address set, files go to post.blinker.cool (the same protocol, no
//! token; files live there 3 days, videos up to 100 MB). A ShareX custom
//! uploader (`CustomUploader`) can be used instead of this protocol.
//!
//! HTTP goes through WinHTTP on Windows and NSURLSession on macOS (system TLS
//! and proxy settings, no extra crates).

use crate::config::{Config, CustomUploader};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinHttp::*;

/// The upload address when none is set.
pub const DEFAULT_URL: &str = "https://post.blinker.cool/api/upload";
pub const DEFAULT_HOST: &str = "post.blinker.cool";
/// The largest file the default server takes.
pub const DEFAULT_MAX_BYTES: u64 = 100 * 1024 * 1024;

/// Where the files go.
#[derive(Clone)]
pub enum Target {
    /// The blinker protocol: an address and a token.
    Blinker { url: String, token: String },
    Custom(CustomUploader),
}

impl Target {
    pub fn of(cfg: &Config) -> Target {
        if let Some(c) = &cfg.upload_custom {
            return Target::Custom(c.clone());
        }
        let url = cfg.upload_url.trim();
        if url.is_empty() {
            Target::Blinker { url: DEFAULT_URL.into(), token: String::new() }
        } else {
            Target::Blinker { url: url.into(), token: cfg.upload_token.trim().into() }
        }
    }

    pub fn is_default(&self) -> bool {
        matches!(self, Target::Blinker { url, .. } if url == DEFAULT_URL)
    }
}

pub struct Job {
    /// 0..=1000 of the body sent.
    pub progress: AtomicU32,
    pub result: Mutex<Option<Result<String, String>>>,
    /// Shown once with the result: what the default server keeps and how long.
    pub note: Option<String>,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
    pub fn result(&self) -> Option<Result<String, String>> {
        self.result.lock().unwrap().clone()
    }
}

/// Uploading is on (there is always somewhere to upload: post.blinker.cool
/// by default).
pub fn configured(cfg: &Config) -> bool {
    cfg.upload_enabled
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// A point on a video's timeline with a note (from the video editor).
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    /// Seconds from the start of the uploaded file.
    pub t: f64,
    pub text: String,
}

/// Uploads `path` in the background; on success the link is copied.
/// `wait` lets the caller hand over a file still being written (autosave).
pub fn start(cfg: &Config, path: PathBuf, wait: Option<Arc<std::sync::atomic::AtomicBool>>) -> Arc<Job> {
    start_with(cfg, path, wait, Vec::new())
}

/// Like `start`, with the video's timeline markers.
pub fn start_with(cfg: &Config, path: PathBuf, wait: Option<Arc<std::sync::atomic::AtomicBool>>, markers: Vec<Marker>) -> Arc<Job> {
    let target = Target::of(cfg);
    // the first upload to our server says what it keeps
    let note = (target.is_default() && !cfg.post_notice_seen).then(|| {
        crate::config::save(&Config { post_notice_seen: true, ..crate::config::load() });
        lf!("{DEFAULT_HOST} хранит файлы 3 дня, видео до 100 мб", "{DEFAULT_HOST} keeps files for 3 days, videos up to 100 mb")
    });
    let job = Arc::new(Job { progress: AtomicU32::new(0), result: Mutex::new(None), note });
    let j = job.clone();
    std::thread::spawn(move || {
        if let Some(flag) = wait {
            let t = std::time::Instant::now();
            while !flag.load(Ordering::Acquire) && t.elapsed() < std::time::Duration::from_secs(10) {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        let r = std::fs::read(&path)
            .map_err(|e| lf!("файл не читается: {e}", "can't read the file: {e}"))
            .and_then(|body| {
                if target.is_default() && body.len() as u64 > DEFAULT_MAX_BYTES {
                    return Err(lf!("{DEFAULT_HOST} принимает файлы до 100 мб", "{DEFAULT_HOST} takes files up to 100 mb"));
                }
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                post(&target, &name, content_type(&path), &body, &markers, &j.progress).map(|p| {
                    log_upload(&p, &path, body.len() as u64);
                    p.url
                })
            });
        if let Ok(link) = &r {
            crate::output::copy_text(link);
        }
        *j.result.lock().unwrap() = Some(r);
    });
    job
}

/// Sends a tiny PNG to check the address and token.
pub fn test(cfg: &Config) -> Result<String, String> {
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([216, 213, 205, 255]));
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        img.as_raw(),
        1,
        1,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    post(&Target::of(cfg), "blinker-test.png", "image/png", &png, &[], &AtomicU32::new(0)).map(|p| p.url)
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn percent(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~ ()".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(windows)]
struct Handle(*mut core::ffi::c_void);
#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

#[cfg(windows)]
fn last_error(what: &str) -> String {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    let hint = match code {
        12007 => crate::i18n::t("сервер не найден"),
        12029 | 12030 => crate::i18n::t("нет соединения с сервером"),
        12002 => crate::i18n::t("сервер не ответил вовремя"),
        12175 | 12045 | 12044 => crate::i18n::t("проблема с сертификатом https"),
        _ => "",
    };
    if hint.is_empty() { lf!("{what}: ошибка {code}", "{what}: error {code}") } else { format!("{what}: {hint}") }
}

/// Markers as the JSON array of the protocol, at most ~6 KB (a header).
fn markers_json(markers: &[Marker]) -> String {
    let mut out = String::from("[");
    for m in markers {
        let item = format!("{{\"t\":{:.2},\"text\":\"{}\"}}", m.t.max(0.0), json_escape(&m.text));
        if out.len() + item.len() > 6000 {
            break;
        }
        if out.len() > 1 {
            out.push(',');
        }
        out.push_str(&item);
    }
    out.push(']');
    out
}

pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// What a server answered to an upload.
struct Posted {
    url: String,
    delete_url: Option<String>,
    /// Unix seconds.
    expires: Option<u64>,
}

fn post(target: &Target, name: &str, ctype: &str, body: &[u8], markers: &[Marker], progress: &AtomicU32) -> Result<Posted, String> {
    let (status, text) = match target {
        Target::Blinker { url, token } => {
            let mut headers = vec![("Content-Type", ctype.to_string()), ("X-Filename", percent(name))];
            if !markers.is_empty() {
                headers.push(("X-Blinker-Markers", percent(&markers_json(markers))));
            }
            if !token.is_empty() {
                headers.push(("Authorization", format!("Bearer {token}")));
            }
            send("POST", url, &headers, body, progress)?
        }
        Target::Custom(c) => send_custom(c, name, ctype, body, progress)?,
    };
    if !(200..300).contains(&status) {
        let msg = json_string(&text, "error").unwrap_or_else(|| text.chars().take(160).collect());
        return Err(match status {
            401 | 403 => crate::i18n::t("сервер не принял токен").to_string(),
            413 => crate::i18n::t("файл слишком большой для сервера").to_string(),
            _ => lf!("сервер ответил {status}: {}", "the server answered {status}: {}", msg.trim()),
        });
    }
    match target {
        Target::Blinker { .. } => {
            let url = json_string(&text, "url").ok_or_else(|| crate::i18n::t("в ответе сервера нет поля \"url\"").to_string())?;
            let expires = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("expires")?.as_u64());
            Ok(Posted { url, delete_url: json_string(&text, "delete_url"), expires })
        }
        Target::Custom(c) => link_from(&c.url_template, &text)
            .map(|url| Posted { url, delete_url: None, expires: None })
            .ok_or_else(|| lf!("в ответе сервера нет ссылки ({})", "no link in the server's answer ({})", c.url_template)),
    }
}

// ---------------------------------------------------------------------------
// What was uploaded: a list kept in the app's folder (the history window
// shows it, with "delete from the server" where the server gave a link)

/// One upload.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Uploaded {
    pub url: String,
    #[serde(default)]
    pub delete_url: Option<String>,
    /// The file on this computer (it may be gone since).
    pub path: PathBuf,
    pub size: u64,
    /// Unix seconds.
    pub at: u64,
    #[serde(default)]
    pub expires: Option<u64>,
}

fn log_file() -> PathBuf {
    crate::config::app_dir().join("uploads.json")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Every upload still worth showing, newest first (expired ones fall away).
pub fn uploads() -> Vec<Uploaded> {
    let mut list: Vec<Uploaded> = std::fs::read(log_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let t = now();
    list.retain(|u| u.expires.is_none_or(|e| e > t));
    list.sort_by(|a, b| b.at.cmp(&a.at));
    list
}

fn save_uploads(list: &[Uploaded]) {
    let _ = std::fs::create_dir_all(crate::config::app_dir());
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(log_file(), json);
    }
}

fn log_upload(p: &Posted, path: &Path, size: u64) {
    // another process may be adding one too: read, add, write at once
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap();
    let mut list = uploads();
    list.insert(0, Uploaded { url: p.url.clone(), delete_url: p.delete_url.clone(), path: path.to_path_buf(), size, at: now(), expires: p.expires });
    list.truncate(500);
    save_uploads(&list);
}

/// Deletes the file from the server (its delete link), then from the list.
pub fn delete_upload(u: &Uploaded) -> Result<(), String> {
    if let Some(del) = &u.delete_url {
        let (status, text) = get(del, &[])?;
        // gone already counts as deleted
        if !(200..300).contains(&status) && status != 404 {
            return Err(json_string(&text, "error").unwrap_or_else(|| lf!("сервер ответил {status}", "the server answered {status}")));
        }
    }
    forget_upload(u);
    Ok(())
}

/// Takes it off the list only (a server without a delete link).
pub fn forget_upload(u: &Uploaded) {
    let mut list = uploads();
    list.retain(|x| x.url != u.url);
    save_uploads(&list);
}

/// ShareX placeholders in a value: the file name.
fn fill(value: &str, name: &str) -> String {
    value.replace("{filename}", name).replace("$filename$", name)
}

/// A ShareX custom uploader: query parameters, headers, then the raw file or
/// a multipart form with it.
fn send_custom(c: &CustomUploader, name: &str, ctype: &str, body: &[u8], progress: &AtomicU32) -> Result<(u32, String), String> {
    if c.url.trim().is_empty() {
        return Err(crate::i18n::t("не указан адрес сервера").into());
    }
    let mut url = fill(c.url.trim(), name);
    for (i, (k, v)) in c.parameters.iter().enumerate() {
        url.push(if i == 0 && !url.contains('?') { '?' } else { '&' });
        url.push_str(&format!("{}={}", query_escape(k), query_escape(&fill(v, name))));
    }
    let mut headers: Vec<(&str, String)> = c.headers.iter().map(|(k, v)| (k.as_str(), fill(v, name))).collect();
    let method = if c.method.trim().is_empty() { "POST".to_string() } else { c.method.trim().to_uppercase() };
    if c.body.eq_ignore_ascii_case("Binary") {
        headers.push(("Content-Type", ctype.to_string()));
        return send(&method, &url, &headers, body, progress);
    }
    let boundary = format!("----blinker{:016x}", body.len() as u64 ^ 0x9e37_79b9_7f4a_7c15);
    let mut form = Vec::with_capacity(body.len() + 1024);
    for (k, v) in &c.arguments {
        form.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{}\r\n", fill(v, name)).as_bytes());
    }
    let field = if c.file_form_name.is_empty() { "file" } else { c.file_form_name.as_str() };
    form.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{}\"\r\nContent-Type: {ctype}\r\n\r\n", name.replace('"', "'"))
            .as_bytes(),
    );
    form.extend_from_slice(body);
    form.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    headers.push(("Content-Type", format!("multipart/form-data; boundary={boundary}")));
    send(&method, &url, &headers, &form, progress)
}

fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// The link out of a server answer by a ShareX URL template: text with
/// `{json:a.b[0].c}`, `$json:a.b$` or `{response}` in it. Empty = the whole answer.
pub fn link_from(template: &str, response: &str) -> Option<String> {
    let template = template.trim();
    if template.is_empty() {
        return Some(response.trim().to_string()).filter(|s| !s.is_empty());
    }
    let json: Option<serde_json::Value> = serde_json::from_str(response).ok();
    let mut out = String::new();
    let mut rest = template;
    while !rest.is_empty() {
        let next = rest.find(['{', '$']);
        let Some(i) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..i]);
        let close = if rest[i..].starts_with('{') { '}' } else { '$' };
        let Some(end) = rest[i + 1..].find(close) else {
            out.push_str(&rest[i..]);
            break;
        };
        let token = &rest[i + 1..i + 1 + end];
        let value = if token == "response" {
            Some(response.trim().to_string())
        } else if let Some(path) = token.strip_prefix("json:") {
            json.as_ref().and_then(|j| json_path(j, path))
        } else {
            None
        };
        match value {
            Some(v) => out.push_str(&v),
            None if token.starts_with("json:") => return None,
            None => out.push_str(&rest[i..i + 2 + end]),
        }
        rest = &rest[i + 2 + end..];
    }
    Some(out).filter(|s| !s.trim().is_empty())
}

/// `data.files[0].url` in a JSON value; strings come without quotes.
fn json_path(v: &serde_json::Value, path: &str) -> Option<String> {
    let mut cur = v;
    for part in path.split('.') {
        let (key, idx) = match part.find('[') {
            Some(b) => (&part[..b], Some(&part[b..])),
            None => (part, None),
        };
        if !key.is_empty() {
            cur = cur.get(key)?;
        }
        if let Some(idx) = idx {
            for i in idx.split(']').filter(|s| !s.is_empty()) {
                cur = cur.get(i.trim_start_matches('[').parse::<usize>().ok()?)?;
            }
        }
    }
    Some(match cur {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => return None,
        other => other.to_string(),
    })
}

/// A GET with `headers`: (status, response text). Also used for the update check.
pub fn get(url: &str, headers: &[(&str, String)]) -> Result<(u32, String), String> {
    send("GET", url, headers, &[], &AtomicU32::new(0))
}

/// Sends `body` with `headers`: (status, response text).
#[cfg(target_os = "macos")]
fn send(method: &str, url: &str, headers: &[(&str, String)], body: &[u8], progress: &AtomicU32) -> Result<(u32, String), String> {
    let (status, text, err) = crate::win::http_request(method, url, headers, body, progress);
    match err {
        None => Ok((status, text)),
        Some(code) => Err(match code {
            -1003 | -1006 => crate::i18n::t("сервер не найден").to_string(),
            -1004 | -1005 | -1009 => crate::i18n::t("нет соединения с сервером").to_string(),
            -1001 => crate::i18n::t("сервер не ответил вовремя").to_string(),
            -1202..=-1200 | -1206..=-1203 => crate::i18n::t("проблема с сертификатом https").to_string(),
            -1000 | -1002 => crate::i18n::t("неверный адрес сервера (нужно https://…)").to_string(),
            _ if text.is_empty() => lf!("отправка: ошибка {code}", "sending: error {code}"),
            _ => text,
        }),
    }
}

/// Sends `body` with `headers`: (status, response text).
#[cfg(windows)]
fn send(method: &str, url: &str, extra_headers: &[(&str, String)], body: &[u8], progress: &AtomicU32) -> Result<(u32, String), String> {
    unsafe {
        let wurl = wide(url);
        let mut host = vec![0u16; 256];
        let mut path = vec![0u16; 2048];
        let mut extra = vec![0u16; 2048];
        let mut uc = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            lpszHostName: host.as_mut_ptr(),
            dwHostNameLength: host.len() as u32,
            lpszUrlPath: path.as_mut_ptr(),
            dwUrlPathLength: path.len() as u32,
            lpszExtraInfo: extra.as_mut_ptr(),
            dwExtraInfoLength: extra.len() as u32,
            ..Default::default()
        };
        if WinHttpCrackUrl(wurl.as_ptr(), 0, 0, &mut uc) == 0 {
            return Err(crate::i18n::t("неверный адрес сервера (нужно https://…)").into());
        }
        let host = String::from_utf16_lossy(&host[..uc.dwHostNameLength as usize]);
        let object = String::from_utf16_lossy(&path[..uc.dwUrlPathLength as usize])
            + &String::from_utf16_lossy(&extra[..uc.dwExtraInfoLength as usize]);
        let secure = uc.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;

        let agent = wide(concat!("Blinker/", env!("CARGO_PKG_VERSION")));
        let session = Handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err(last_error("WinHTTP"));
        }
        WinHttpSetTimeouts(session.0, 10_000, 15_000, 120_000, 60_000);
        let whost = wide(&host);
        let conn = Handle(WinHttpConnect(session.0, whost.as_ptr(), uc.nPort, 0));
        if conn.0.is_null() {
            return Err(last_error(crate::i18n::t("соединение")));
        }
        let (verb, wobj) = (wide(method), wide(if object.is_empty() { "/" } else { &object }));
        let req = Handle(WinHttpOpenRequest(conn.0, verb.as_ptr(), wobj.as_ptr(), null(), null(), null(), if secure { WINHTTP_FLAG_SECURE } else { 0 }));
        if req.0.is_null() {
            return Err(last_error(crate::i18n::t("запрос")));
        }
        let headers: String = extra_headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        let wh = wide(&headers);
        if WinHttpSendRequest(req.0, wh.as_ptr(), u32::MAX, null(), 0, body.len() as u32, 0) == 0 {
            return Err(last_error(crate::i18n::t("отправка")));
        }
        let mut sent = 0usize;
        for chunk in body.chunks(256 * 1024) {
            let mut n = 0u32;
            if WinHttpWriteData(req.0, chunk.as_ptr() as _, chunk.len() as u32, &mut n) == 0 {
                return Err(last_error(crate::i18n::t("отправка")));
            }
            sent += n as usize;
            progress.store((sent as f64 / body.len().max(1) as f64 * 1000.0) as u32, Ordering::Relaxed);
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(last_error(crate::i18n::t("ответ")));
        }
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as _,
            &mut len,
            null_mut(),
        );
        let mut resp = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut n = 0u32;
            if WinHttpReadData(req.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut n) == 0 || n == 0 {
                break;
            }
            resp.extend_from_slice(&buf[..n as usize]);
            if resp.len() > 1 << 20 {
                break;
            }
        }
        Ok((status, String::from_utf8_lossy(&resp).into_owned()))
    }
}

/// Value of a top-level string field in a small JSON object (enough for the
/// upload response; no full JSON parser needed).
pub fn json_string(json: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let mut rest = &json[json.find(&pat)? + pat.len()..];
    rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?).unwrap_or('?'));
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn sharex_templates() {
        let r = r#"{"data":{"files":[{"url":"https://x.y/a.png"}]},"ok":true}"#;
        assert_eq!(super::link_from("{json:data.files[0].url}", r).as_deref(), Some("https://x.y/a.png"));
        assert_eq!(super::link_from("$json:data.files[0].url$", r).as_deref(), Some("https://x.y/a.png"));
        assert_eq!(super::link_from("https://i.y/{json:ok}", r).as_deref(), Some("https://i.y/true"));
        assert_eq!(super::link_from("", " https://plain \n").as_deref(), Some("https://plain"));
        assert_eq!(super::link_from("{response}", "https://r").as_deref(), Some("https://r"));
        assert_eq!(super::link_from("{json:missing}", r), None);
    }

    #[test]
    fn markers_as_json() {
        let m = vec![super::Marker { t: 1.5, text: "a \"b\"\nc".into() }, super::Marker { t: 3.0, text: String::new() }];
        assert_eq!(super::markers_json(&m), r#"[{"t":1.50,"text":"a \"b\"\nc"},{"t":3.00,"text":""}]"#);
    }

    #[test]
    fn parses_response() {
        let j = r#"{ "url" : "https:\/\/s.example.com\/a b", "delete_url": "x" }"#;
        assert_eq!(super::json_string(j, "url").as_deref(), Some("https://s.example.com/a b"));
        assert_eq!(super::json_string(r#"{"error":"bad \"token\""}"#, "error").as_deref(), Some("bad \"token\""));
        assert_eq!(super::json_string("{}", "url"), None);
    }
}

#[cfg(test)]
mod live {
    /// Uploads to a local test server: `BLINKER_TEST_UPLOAD` = URL.
    #[test]
    fn upload_local() {
        let Ok(url) = std::env::var("BLINKER_TEST_UPLOAD") else { return };
        let mut cfg = crate::config::Config { upload_url: url, upload_token: "test-token".into(), ..Default::default() };
        cfg.upload_custom = None;
        let r = super::test(&cfg);
        eprintln!("ok token: {r:?}");
        assert!(r.as_ref().is_ok_and(|u| u.starts_with("http")), "{r:?}");
        cfg.upload_token = "wrong".into();
        let r = super::test(&cfg);
        eprintln!("bad token: {r:?}");
        assert!(r.is_err());
    }
}
