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
//! <raw file bytes>
//!
//! 200 / 201  {"url": "https://…", "delete_url": "https://…" (optional)}
//! 4xx / 5xx  {"error": "message"}  (or any text)
//! ```
//!
//! HTTP goes through WinHTTP (system TLS and proxy settings, no extra crates).

use crate::config::Config;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Networking::WinHttp::*;

pub struct Job {
    /// 0..=1000 of the body sent.
    pub progress: AtomicU32,
    pub result: Mutex<Option<Result<String, String>>>,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }
    pub fn result(&self) -> Option<Result<String, String>> {
        self.result.lock().unwrap().clone()
    }
}

pub fn configured(cfg: &Config) -> bool {
    !cfg.upload_url.trim().is_empty()
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// Uploads `path` in the background; on success the link is copied.
/// `wait` lets the caller hand over a file still being written (autosave).
pub fn start(cfg: &Config, path: PathBuf, wait: Option<Arc<std::sync::atomic::AtomicBool>>) -> Arc<Job> {
    let job = Arc::new(Job { progress: AtomicU32::new(0), result: Mutex::new(None) });
    let (j, url, token) = (job.clone(), cfg.upload_url.trim().to_string(), cfg.upload_token.trim().to_string());
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
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                post(&url, &token, &name, content_type(&path), &body, &j.progress)
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
    post(cfg.upload_url.trim(), cfg.upload_token.trim(), "blinker-test.png", "image/png", &png, &AtomicU32::new(0))
}

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

struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

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

fn post(url: &str, token: &str, name: &str, ctype: &str, body: &[u8], progress: &AtomicU32) -> Result<String, String> {
    if url.is_empty() {
        return Err(crate::i18n::t("не указан адрес сервера").into());
    }
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
        let (verb, wobj) = (wide("POST"), wide(if object.is_empty() { "/" } else { &object }));
        let req = Handle(WinHttpOpenRequest(conn.0, verb.as_ptr(), wobj.as_ptr(), null(), null(), null(), if secure { WINHTTP_FLAG_SECURE } else { 0 }));
        if req.0.is_null() {
            return Err(last_error(crate::i18n::t("запрос")));
        }
        let mut headers = format!("Content-Type: {ctype}\r\nX-Filename: {}\r\n", percent(name));
        if !token.is_empty() {
            headers += &format!("Authorization: Bearer {token}\r\n");
        }
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
        let text = String::from_utf8_lossy(&resp).into_owned();
        if !(200..300).contains(&status) {
            let msg = json_string(&text, "error").unwrap_or_else(|| text.chars().take(160).collect());
            return Err(match status {
                401 | 403 => crate::i18n::t("сервер не принял токен").to_string(),
                413 => crate::i18n::t("файл слишком большой для сервера").to_string(),
                _ => lf!("сервер ответил {status}: {}", "the server answered {status}: {}", msg.trim()),
            });
        }
        json_string(&text, "url").ok_or_else(|| crate::i18n::t("в ответе сервера нет поля \"url\"").to_string())
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
        let r = super::test(&cfg);
        eprintln!("ok token: {r:?}");
        assert!(r.as_ref().is_ok_and(|u| u.contains("blinker-test.png")), "{r:?}");
        cfg.upload_token = "wrong".into();
        let r = super::test(&cfg);
        eprintln!("bad token: {r:?}");
        assert!(r.is_err());
    }
}
