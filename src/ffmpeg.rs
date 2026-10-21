//! ffmpeg: where it is (the folder from the settings, our own download, the
//! usual install places, PATH) and getting it in one click: a static build
//! from github.com/eugeneware/ffmpeg-static into blinker's own folder.
//! blinker works without it; with it "for Telegram" uses a tuned NVENC /
//! x264 recipe.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The ffmpeg-static release the download takes.
const BUILD: &str = "b6.1.1";

fn exe() -> &'static str {
    if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }
}

/// This system's file of the release (gzipped), if it has one.
fn asset() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("ffmpeg-darwin-arm64.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("ffmpeg-darwin-x64.gz")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("ffmpeg-win32-x64.gz")
    } else {
        None
    }
}

/// Where the downloaded ffmpeg lives.
pub fn own_dir() -> PathBuf {
    crate::config::app_dir().join("ffmpeg")
}

/// The last answer of `find` and when it was given: the settings ask every
/// frame, the disk is looked at every couple of seconds.
static FOUND: Mutex<Option<(std::time::Instant, Option<PathBuf>)>> = Mutex::new(None);

/// ffmpeg to use: the folder set in the settings, else the first one found.
pub fn find() -> Option<PathBuf> {
    let mut found = FOUND.lock().unwrap();
    if let Some((at, p)) = &*found
        && at.elapsed() < std::time::Duration::from_secs(2)
    {
        return p.clone();
    }
    let p = look();
    *found = Some((std::time::Instant::now(), p.clone()));
    p
}

/// Forget the last answer (after a download or a new folder).
pub fn forget() {
    *FOUND.lock().unwrap() = None;
}

fn look() -> Option<PathBuf> {
    let dir = crate::config::load().ffmpeg_dir;
    let dir = dir.trim();
    if !dir.is_empty() {
        let p = Path::new(dir);
        let f = if p.is_file() { p.to_path_buf() } else { p.join(exe()) };
        if f.is_file() {
            return Some(f);
        }
    }
    detect()
}

/// Looks in our folder, the places installers put it, then PATH (apps
/// started from Finder get a bare PATH, so Homebrew's folders are named).
pub fn detect() -> Option<PathBuf> {
    let mut places = vec![own_dir().join(exe())];
    if cfg!(windows) {
        places.push(PathBuf::from(r"C:\ffmpeg\bin\ffmpeg.exe"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            places.push(PathBuf::from(&local).join(r"Microsoft\WinGet\Links\ffmpeg.exe"));
        }
        if let Some(home) = dirs::home_dir() {
            places.push(home.join(r"scoop\shims\ffmpeg.exe"));
        }
        places.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin\ffmpeg.exe"));
        for pf in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(d) = std::env::var_os(pf) {
                places.push(PathBuf::from(d).join(r"ffmpeg\bin\ffmpeg.exe"));
            }
        }
    } else {
        places.extend(["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/opt/local/bin/ffmpeg"].map(PathBuf::from));
    }
    if let Some(p) = places.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe())).find(|f| f.is_file()))
}

/// ffmpeg being downloaded.
pub struct Download {
    /// (bytes so far, in all)
    pub progress: Mutex<(u64, u64)>,
    pub result: Mutex<Option<Result<PathBuf, String>>>,
}

impl Download {
    pub fn fraction(&self) -> f32 {
        let (a, b) = *self.progress.lock().unwrap();
        if b == 0 { 0.0 } else { (a as f32 / b as f32).clamp(0.0, 1.0) }
    }
    pub fn result(&self) -> Option<Result<PathBuf, String>> {
        self.result.lock().unwrap().clone()
    }
}

/// Downloads ffmpeg into `own_dir` in the background; `repaint` on progress.
pub fn download(repaint: impl Fn() + Send + 'static) -> Arc<Download> {
    let job = Arc::new(Download { progress: Mutex::new((0, 0)), result: Mutex::new(None) });
    let j = job.clone();
    std::thread::spawn(move || {
        let r = fetch(&j, &repaint);
        if let Err(e) = &r {
            crate::config::log::error(format!("ffmpeg download: {e}"));
        }
        *j.result.lock().unwrap() = Some(r);
        repaint();
    });
    job
}

fn fetch(job: &Download, repaint: &dyn Fn()) -> Result<PathBuf, String> {
    let name = asset().ok_or_else(|| crate::i18n::l("для этой системы готового ffmpeg нет", "there is no ready ffmpeg for this system").to_string())?;
    let url = format!("https://github.com/eugeneware/ffmpeg-static/releases/download/{BUILD}/{name}");
    let size = remote_size(&url).unwrap_or(0);
    *job.progress.lock().unwrap() = (0, size);
    let dir = own_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let gz = dir.join(format!("{name}.part"));
    let _ = std::fs::remove_file(&gz);
    crate::update::download(&url, &gz, size, |done| {
        *job.progress.lock().unwrap() = (done, size);
        repaint();
    })?;
    // unpack next to the final name, then put it in place
    let tmp = dir.join(format!("{}.new", exe()));
    let unpack = || -> std::io::Result<()> {
        let mut src = flate2::read::GzDecoder::new(std::io::BufReader::new(std::fs::File::open(&gz)?));
        let mut dst = std::fs::File::create(&tmp)?;
        std::io::copy(&mut src, &mut dst)?;
        Ok(())
    };
    let r = unpack();
    let _ = std::fs::remove_file(&gz);
    r.map_err(|e| lf!("не распаковалось: {e}", "could not unpack: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    // Apple silicon runs signed code only: an ad hoc signature is enough
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&tmp).output();
    }
    let dst = dir.join(exe());
    let _ = std::fs::remove_file(&dst);
    std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
    if !works(&dst) {
        let _ = std::fs::remove_file(&dst);
        return Err(crate::i18n::l("скачанный ffmpeg не запускается", "the downloaded ffmpeg doesn't run").into());
    }
    forget();
    Ok(dst)
}

/// The size of a file to download (curl follows GitHub's redirects).
fn remote_size(url: &str) -> Option<u64> {
    let mut cmd = std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" });
    cmd.args(["-sIL", url]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string()))
        .filter_map(|v| v.parse::<u64>().ok())
        .next_back()
}

/// `ffmpeg -version` answers.
pub fn works(ff: &Path) -> bool {
    let mut cmd = std::process::Command::new(ff);
    cmd.arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.status().is_ok_and(|s| s.success())
}
