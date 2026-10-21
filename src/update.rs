//! Updates. Is there a newer blinker? The latest GitHub release (the one
//! blinker.cool offers for download) against this build: asked in the
//! background at most every few hours, the answer cached on disk. And
//! installing it in one click: download, check, swap the app, start again.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const REPO_URL: &str = "https://github.com/justkiddingxd/blinker";
/// Where an update is downloaded (with the Mac guide).
pub const SITE_URL: &str = "https://blinker.cool";
const LATEST_API: &str = "https://api.github.com/repos/justkiddingxd/blinker/releases/latest";
/// GitHub allows 60 unauthenticated requests an hour: plenty, but no need to ask often.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// This build's version.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// "v1.2.10" -> [1, 2, 10] (anything after a number's digits is ignored).
fn parse(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|part| part.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
        .collect()
}

/// `tag` is a later version than this build.
fn is_newer(tag: &str) -> bool {
    let (a, b) = (parse(tag), parse(current()));
    let n = a.len().max(b.len());
    let pad = |mut v: Vec<u64>| {
        v.resize(n, 0);
        v
    };
    pad(a) > pad(b)
}

fn cache_path() -> std::path::PathBuf {
    crate::config::cache_dir().join("update-check")
}

/// (when it was asked, the latest release's tag)
fn cached() -> Option<(SystemTime, String)> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    let mut lines = text.lines();
    let secs: u64 = lines.next()?.trim().parse().ok()?;
    let tag = lines.next()?.trim().to_string();
    Some((UNIX_EPOCH + Duration::from_secs(secs), tag))
}

fn remember(tag: &str) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::create_dir_all(crate::config::cache_dir());
    let _ = std::fs::write(cache_path(), format!("{secs}\n{tag}\n"));
}

/// The release file for this system.
#[cfg(target_os = "macos")]
const ASSET: &str = "Blinker-macos.zip";
#[cfg(windows)]
const ASSET: &str = "blinker.exe";

struct Release {
    tag: String,
    /// (download address, size in bytes) of `ASSET`, when the release has it.
    asset: Option<(String, u64)>,
}

/// The latest release, straight from GitHub.
fn fetch_release() -> Result<Release, String> {
    let headers = [("Accept", "application/vnd.github+json".to_string())];
    let (status, body) = crate::upload::get(LATEST_API, &headers)?;
    if status != 200 {
        return Err(format!("GitHub answered {status}"));
    }
    let tag = crate::upload::json_string(&body, "tag_name").ok_or_else(|| "no tag_name in the answer".to_string())?;
    Ok(Release { tag, asset: find_asset(&body, ASSET) })
}

/// (download address, size) of the asset `name` in a release's JSON:
/// {"name": "...", ... "size": N, ... "browser_download_url": "..."}.
fn find_asset(body: &str, name: &str) -> Option<(String, u64)> {
    let at = body.find(&format!("\"name\":\"{name}\"")).or_else(|| body.find(&format!("\"name\": \"{name}\"")))?;
    let rest = &body[at..];
    let url = crate::upload::json_string(rest, "browser_download_url")?;
    let i = rest.find("\"size\"")?;
    let digits: String = rest[i + 6..].chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
    Some((url, digits.parse().ok()?))
}

fn fetch() -> Result<String, String> {
    fetch_release().map(|r| r.tag)
}

/// A newer version, when there is one. Filled from the cache right away and
/// refreshed in the background when the cache is old; `ctx` repaints then.
pub fn watch(ctx: &egui::Context) -> Arc<Mutex<Option<String>>> {
    let newer = |tag: &str| is_newer(tag).then(|| tag.trim_start_matches(['v', 'V']).to_string());
    let cache = cached();
    let slot = Arc::new(Mutex::new(cache.as_ref().and_then(|(_, tag)| newer(tag))));
    let fresh = cache.is_some_and(|(at, _)| at.elapsed().is_ok_and(|age| age < EVERY));
    if !fresh {
        let (slot, ctx) = (slot.clone(), ctx.clone());
        std::thread::spawn(move || match fetch() {
            Ok(tag) => {
                remember(&tag);
                *slot.lock().unwrap() = newer(&tag);
                ctx.request_repaint();
            }
            Err(e) => crate::config::log::error(format!("update check: {e}")),
        });
    }
    slot
}

// ---------------------------------------------------------------------------
// Installing

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Bytes so far / in all.
    Downloading(u64, u64),
    Installing,
    /// Installed: the app starts again.
    Restarting,
    Failed(String),
}

/// One update being installed (the settings show its stage).
pub struct Install {
    pub stage: Mutex<Stage>,
}

impl Install {
    pub fn stage(&self) -> Stage {
        self.stage.lock().unwrap().clone()
    }
}

/// Downloads the latest release and puts it in place of this app, then
/// restarts blinker. Runs in the background; `ctx` repaints on progress.
pub fn install(ctx: &egui::Context) -> Arc<Install> {
    let job = Arc::new(Install { stage: Mutex::new(Stage::Downloading(0, 0)) });
    let (j, ctx) = (job.clone(), ctx.clone());
    std::thread::spawn(move || {
        let result = run_install(&j, &ctx);
        let stage = match result {
            Ok(()) => Stage::Restarting,
            Err(e) => {
                crate::config::log::error(format!("update: {e}"));
                Stage::Failed(e)
            }
        };
        let restart = stage == Stage::Restarting;
        *j.stage.lock().unwrap() = stage;
        ctx.request_repaint();
        if restart {
            restart_app();
        }
    });
    job
}

fn set(job: &Install, ctx: &egui::Context, stage: Stage) {
    *job.stage.lock().unwrap() = stage;
    ctx.request_repaint();
}

fn run_install(job: &Install, ctx: &egui::Context) -> Result<(), String> {
    let release = fetch_release()?;
    if !is_newer(&release.tag) {
        return Err(crate::i18n::l("это уже последняя версия", "this is already the latest version").into());
    }
    let (url, size) = release.asset.ok_or_else(|| lf!("в релизе нет файла {ASSET}", "the release has no {ASSET}"))?;
    let file = download_path()?;
    let _ = std::fs::remove_file(&file);
    download(&url, &file, size, |done| set(job, ctx, Stage::Downloading(done, size)))?;
    set(job, ctx, Stage::Installing);
    let r = put_in_place(&file);
    if r.is_err() {
        let _ = std::fs::remove_file(&file);
    }
    r
}

/// `curl` ships with macOS and Windows 10+: follows GitHub's redirects, uses
/// the system's TLS and proxy. Progress comes from the file's size.
fn download(url: &str, dst: &Path, size: u64, progress: impl Fn(u64)) -> Result<(), String> {
    let mut cmd = std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" });
    cmd.args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "-o"]).arg(dst).arg(url);
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("curl: {e}"))?;
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        progress(std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0));
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut e, &mut err);
        }
        return Err(lf!("не скачалось: {}", "the download failed: {}", err.trim()));
    }
    let got = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
    if size > 0 && got != size {
        return Err(lf!("скачалось не целиком ({got} из {size} байт)", "the download is incomplete ({got} of {size} bytes)"));
    }
    progress(got);
    Ok(())
}

/// The app bundle this process runs from (…/Blinker.app).
#[cfg(target_os = "macos")]
fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

#[cfg(target_os = "macos")]
fn download_path() -> Result<PathBuf, String> {
    let dir = crate::config::cache_dir().join("update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(ASSET))
}

/// Windows: next to the running exe (the swap is a rename on one volume).
#[cfg(windows)]
fn download_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.with_extension("exe.new"))
}

#[cfg(target_os = "macos")]
fn sh(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = std::process::Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if out.status.success() { Ok(text) } else { Err(format!("{cmd}: {}", text.trim())) }
}

/// The code signing requirement of an app ("identifier … and certificate leaf = …").
#[cfg(target_os = "macos")]
fn requirement(app: &Path) -> Option<String> {
    let text = sh("/usr/bin/codesign", &["-d".as_ref(), "-r-".as_ref(), app.as_os_str()]).ok()?;
    text.lines().find_map(|l| l.strip_prefix("designated => ").map(str::to_string))
}

/// Unpacks the new app next to the running one, checks it is signed by the
/// same certificate, then swaps the two (rolled back if the swap fails).
#[cfg(target_os = "macos")]
fn put_in_place(zip: &Path) -> Result<(), String> {
    let app = bundle().ok_or_else(|| crate::i18n::l("blinker запущен не из Blinker.app", "blinker doesn't run from Blinker.app").to_string())?;
    replace_app(&app, zip)
}

#[cfg(target_os = "macos")]
fn replace_app(app: &Path, zip: &Path) -> Result<(), String> {
    let app = app.to_path_buf();
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Err(crate::i18n::l(
            "перенеси Blinker в «Программы» и запусти оттуда: из «Загрузок» macOS не даёт его заменить",
            "move Blinker to Applications and open it from there: macOS doesn't let it be replaced in Downloads",
        )
        .into());
    }
    let parent = app.parent().ok_or("no parent folder")?;
    let work = parent.join(format!(".blinker-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| lf!("нет прав на папку {}: {e}", "no access to {}: {e}", parent.display()))?;
    let r = swap_in(&app, zip, &work);
    if r.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    r
}

/// The new app from `zip` into `work`, checked, then in place of `app`.
#[cfg(target_os = "macos")]
fn swap_in(app: &Path, zip: &Path, work: &Path) -> Result<(), String> {
    let unpacked = work.join("new");
    sh("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), zip.as_os_str(), unpacked.as_os_str()])?;
    let new_app = unpacked.join("Blinker.app");
    if !new_app.is_dir() {
        return Err(crate::i18n::l("в архиве нет Blinker.app", "Blinker.app is not in the archive").into());
    }
    sh("/usr/bin/codesign", &["--verify".as_ref(), "--strict".as_ref(), new_app.as_os_str()])?;
    // signed with a certificate: the update must carry the same one
    if let Some(mine) = requirement(app).filter(|r| r.contains("certificate"))
        && requirement(&new_app).as_deref() != Some(mine.as_str())
    {
        return Err(crate::i18n::l("у обновления другая подпись: не ставлю", "the update is signed differently: not installing").into());
    }
    let old = work.join("old.app");
    std::fs::rename(app, &old).map_err(|e| lf!("не заменить {}: {e}", "can't replace {}: {e}", app.display()))?;
    if let Err(e) = std::fs::rename(&new_app, app) {
        let _ = std::fs::rename(&old, app);
        return Err(lf!("не заменить {}: {e}", "can't replace {}: {e}", app.display()));
    }
    // the running processes still use the old copy: it is deleted next start
    let _ = std::fs::remove_file(zip);
    Ok(())
}

/// Renames the running exe away (Windows allows that) and puts the new one
/// in its place.
#[cfg(windows)]
fn put_in_place(new: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| lf!("не заменить {}: {e}", "can't replace {}: {e}", exe.display()))?;
    if let Err(e) = std::fs::rename(new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(lf!("не заменить {}: {e}", "can't replace {}: {e}", exe.display()));
    }
    Ok(())
}

/// Leftovers of an update: the old copy of the app.
pub fn clean_up() {
    #[cfg(target_os = "macos")]
    if let Some(parent) = bundle().as_deref().and_then(Path::parent) {
        for e in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with(".blinker-update-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
        let _ = std::fs::remove_file(exe.with_extension("exe.new"));
    }
}

/// Asks the tray to quit and start again (or starts the new app itself when
/// there is no tray), then leaves: this process runs the old copy.
fn restart_app() {
    if crate::win::window_class_exists("BlinkerShell") {
        crate::win::post_to_window_class("BlinkerShell", crate::runtime::WM_RESTART);
    } else {
        relaunch();
    }
    std::thread::sleep(Duration::from_millis(300));
    std::process::exit(0);
}

/// Starts blinker again from its (new) files; it waits for this instance to end.
pub fn relaunch() {
    #[cfg(target_os = "macos")]
    if let Some(app) = bundle() {
        let _ = std::process::Command::new("/usr/bin/open").arg("-n").arg(&app).args(["--args", "--relaunch"]).spawn();
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--relaunch").spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        use super::parse;
        assert_eq!(parse("v1.2.10"), vec![1, 2, 10]);
        assert_eq!(parse("0.3.0-beta"), vec![0, 3, 0]);
        let newer = |a: &str, b: &str| {
            let (x, y) = (parse(a), parse(b));
            let n = x.len().max(y.len());
            let (mut x, mut y) = (x, y);
            x.resize(n, 0);
            y.resize(n, 0);
            x > y
        };
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("v0.1", "0.1.0"));
        assert!(newer("1.0", "0.9.9"));
    }

    /// Asks GitHub for real: `BLINKER_TEST_UPDATE=1`.
    #[test]
    fn live() {
        if std::env::var("BLINKER_TEST_UPDATE").is_err() {
            return;
        }
        let tag = super::fetch().expect("fetch");
        eprintln!("latest release: {tag}, this build {}, newer: {}", super::current(), super::is_newer(&tag));
    }

    /// The real v0.1.0 release: finds blinker.exe and downloads it.
    /// `BLINKER_TEST_DOWNLOAD` = a folder.
    #[test]
    fn release_asset_and_download() {
        let Ok(dir) = std::env::var("BLINKER_TEST_DOWNLOAD") else { return };
        let (status, body) = crate::upload::get(super::LATEST_API, &[]).unwrap();
        assert_eq!(status, 200);
        let (url, size) = super::find_asset(&body, "blinker.exe").expect("asset");
        eprintln!("{url} {size} bytes");
        let dst = std::path::Path::new(&dir).join("blinker.exe");
        let seen = std::sync::atomic::AtomicU64::new(0);
        super::download(&url, &dst, size, |n| seen.store(n, std::sync::atomic::Ordering::Relaxed)).unwrap();
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), size);
        assert!(super::find_asset(&body, "nothing.zip").is_none());
    }

    /// Swaps a copy of Blinker.app for a zipped build: `BLINKER_TEST_SWAP` =
    /// a folder with current/Blinker.app, new.zip (same signature) and adhoc.zip.
    #[cfg(target_os = "macos")]
    #[test]
    fn swap_app() {
        let Ok(dir) = std::env::var("BLINKER_TEST_SWAP") else { return };
        let dir = std::path::PathBuf::from(dir);
        let app = dir.join("current/Blinker.app");
        let before = std::fs::metadata(app.join("Contents/MacOS/blinker")).unwrap().modified().unwrap();
        // signed by another hand: refused, the app stays
        let e = super::replace_app(&app, &dir.join("adhoc.zip")).unwrap_err();
        eprintln!("ad hoc update: {e}");
        assert!(app.join("Contents/Info.plist").is_file());
        super::replace_app(&app, &dir.join("new.zip")).expect("swap");
        let after = std::fs::metadata(app.join("Contents/MacOS/blinker")).unwrap().modified().unwrap();
        assert_ne!(before, after, "the app was not replaced");
        assert_eq!(super::requirement(&app).unwrap_or_default().contains("certificate"), true);
    }
}
