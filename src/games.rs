//! Which game is running, for the Discord status ("Clipping Dota 2").
//! Windows: a known game's process (in front or not), or a full-screen
//! window in front whose program sits in a game library (Steam, Epic,
//! Riot, Xbox, GOG). Libraries hold tools too (Wallpaper Engine runs all
//! the time), so a program of theirs counts only while it plays in front.

/// Process names of games that don't sit in a library folder we know, or
/// whose folder name is not the game's.
#[cfg(windows)]
const KNOWN: &[(&str, &str)] = &[
    ("dota2.exe", "Dota 2"),
    ("cs2.exe", "Counter-Strike 2"),
    ("csgo.exe", "CS:GO"),
    ("valorant-win64-shipping.exe", "VALORANT"),
    ("league of legends.exe", "League of Legends"),
    ("fortniteclient-win64-shipping.exe", "Fortnite"),
    ("r5apex.exe", "Apex Legends"),
    ("r5apex_dx12.exe", "Apex Legends"),
    ("overwatch.exe", "Overwatch 2"),
    ("gta5.exe", "GTA V"),
    ("gta5_enhanced.exe", "GTA V"),
    ("rocketleague.exe", "Rocket League"),
    ("eldenring.exe", "ELDEN RING"),
    ("cyberpunk2077.exe", "Cyberpunk 2077"),
    ("rustclient.exe", "Rust"),
    ("tslgame.exe", "PUBG"),
    ("project8.exe", "Deadlock"),
    ("marvel-win64-shipping.exe", "Marvel Rivals"),
    ("destiny2.exe", "Destiny 2"),
    ("escapefromtarkov.exe", "Escape from Tarkov"),
    ("wow.exe", "World of Warcraft"),
    ("robloxplayerbeta.exe", "Roblox"),
    ("rainbowsix.exe", "Rainbow Six Siege"),
    ("cod.exe", "Call of Duty"),
    ("bg3.exe", "Baldur's Gate 3"),
    ("bg3_dx11.exe", "Baldur's Gate 3"),
    ("helldivers2.exe", "HELLDIVERS 2"),
    ("osu!.exe", "osu!"),
    ("witcher3.exe", "The Witcher 3"),
    ("minecraft.windows.exe", "Minecraft"),
    ("deadbydaylight-win64-shipping.exe", "Dead by Daylight"),
    ("pathofexile_x64.exe", "Path of Exile"),
    ("pathofexile2.exe", "Path of Exile 2"),
    ("hearthstone.exe", "Hearthstone"),
    ("starrail.exe", "Honkai: Star Rail"),
    ("genshinimpact.exe", "Genshin Impact"),
    ("zenlesszonezero.exe", "Zenless Zone Zero"),
    ("warthunder.exe", "War Thunder"),
    ("aces.exe", "War Thunder"),
    ("worldoftanks.exe", "World of Tanks"),
];

/// Library folders: the game's name is the folder right after them.
#[cfg(windows)]
const LIBRARIES: &[&str] = &[r"\steamapps\common\", r"\epic games\", r"\riot games\", r"\xboxgames\", r"\gog galaxy\games\"];

/// Helpers that live next to games and are not one.
#[cfg(windows)]
fn helper(exe: &str) -> bool {
    ["crash", "anticheat", "easyanti", "battleye", "launcher", "helper", "setup", "redist", "update", "report", "unins", "steamworks", "vc_redist", "dxsetup", "riotclient", "epicgames"]
        .iter()
        .any(|w| exe.contains(w))
}

/// A known game's name for a program's full path.
#[cfg(windows)]
fn known(path: &str) -> Option<String> {
    let exe = path.to_lowercase().rsplit('\\').next().unwrap_or("").to_string();
    KNOWN.iter().find(|(e, _)| *e == exe).map(|(_, n)| (*n).to_string())
}

/// The game for a program's full path: a known one, or the library folder
/// it sits in.
#[cfg(windows)]
fn game_of(path: &str) -> Option<String> {
    if let Some(name) = known(path) {
        return Some(name);
    }
    let lower = path.to_lowercase();
    let exe = lower.rsplit('\\').next().unwrap_or("");
    if helper(exe) {
        return None;
    }
    let lib = LIBRARIES.iter().find_map(|l| lower.find(l).map(|i| i + l.len()))?;
    let folder = path[lib..].split('\\').next()?.trim();
    // "Steamworks Shared", "SteamVR" and the like hold tools
    if folder.is_empty() || folder.to_lowercase().starts_with("steam") {
        return None;
    }
    Some(folder.to_string())
}

/// A game found running.
#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    pub name: String,
    /// Its icon on Discord's CDN, when Discord knows the game.
    pub icon: Option<String>,
}

/// The running game: the one in front, else a known one in the background.
#[cfg(windows)]
pub fn running() -> Option<Game> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    let path_of = |pid: u32| -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(h);
            ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    };
    let db = db::get();
    // Discord's list first (it has the icon), then ours
    let named = |path: &str| -> Option<Game> {
        db.as_ref()
            .and_then(|d| d.find(path))
            .or_else(|| known(path).map(|name| Game { icon: db.as_ref().and_then(|d| d.icon_of(&name)), name }))
    };
    let window = unsafe { GetForegroundWindow() };
    let mut front = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut front) };
    if front != 0
        && let Some(path) = path_of(front)
    {
        if let Some(g) = named(&path) {
            return Some(g);
        }
        if full_screen(window)
            && let Some(name) = game_of(&path)
        {
            return Some(Game { icon: db.as_ref().and_then(|d| d.icon_of(&name)), name });
        }
    }
    let mut pids = vec![0u32; 4096];
    let mut bytes = 0u32;
    if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes) } == 0 {
        return None;
    }
    pids.truncate(bytes as usize / 4);
    pids.into_iter().filter(|&p| p != 0 && p != front).find_map(|p| path_of(p).and_then(|p| named(&p)))
}

/// The window covers its whole monitor (full screen or borderless).
#[cfg(windows)]
fn full_screen(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    unsafe {
        let mut r = std::mem::zeroed();
        if hwnd.is_null() || GetWindowRect(hwnd, &mut r) == 0 {
            return false;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

#[cfg(not(windows))]
pub fn running() -> Option<Game> {
    None
}

/// Discord's list of games it detects (24 000 of them, with their program
/// names and icons), kept in the cache folder as a small table and fetched
/// again once a week.
#[cfg_attr(not(windows), allow(dead_code))]
mod db {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    const URL: &str = "https://discord.com/api/v9/applications/detectable";
    const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
    const OS: &str = if cfg!(windows) { "win32" } else { "darwin" };
    /// Program names too plain to say which game it is.
    const PLAIN: &[&str] = &[
        "game.exe", "launcher.exe", "client.exe", "start.exe", "main.exe", "app.exe", "play.exe", "run.exe", "server.exe",
        "setup.exe", "update.exe", "java.exe", "javaw.exe", "python.exe", "pythonw.exe", "node.exe", "electron.exe", "nw.exe",
    ];

    pub struct Db {
        /// Program file name → (path ending to match, game index).
        by_file: HashMap<String, Vec<(String, usize)>>,
        /// (name, Discord application id, icon hash)
        games: Vec<(String, String, String)>,
    }

    impl Db {
        /// The game whose program is at `path`.
        pub fn find(&self, path: &str) -> Option<super::Game> {
            let p = path.to_lowercase().replace('\\', "/");
            let file = p.rsplit('/').next()?;
            if PLAIN.contains(&file) {
                return None;
            }
            // the longest matching ending is the most precise
            let (_, gi) = self
                .by_file
                .get(file)?
                .iter()
                .filter(|(end, _)| !end.contains('/') || p.ends_with(&format!("/{end}")))
                .max_by_key(|(end, _)| end.len())?;
            let (name, id, icon) = &self.games[*gi];
            Some(super::Game { name: name.clone(), icon: url(id, icon) })
        }

        /// The icon of a game by its name.
        pub fn icon_of(&self, name: &str) -> Option<String> {
            self.games.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name)).and_then(|(_, id, icon)| url(id, icon))
        }
    }

    fn url(id: &str, icon: &str) -> Option<String> {
        (!icon.is_empty()).then(|| format!("https://cdn.discordapp.com/app-icons/{id}/{icon}.png?size=128"))
    }

    fn table() -> PathBuf {
        crate::config::cache_dir().join("discord-games.tsv")
    }

    static LOADED: Mutex<Option<(SystemTime, Arc<Db>)>> = Mutex::new(None);
    static FETCHING: AtomicBool = AtomicBool::new(false);

    /// The list as it is now (fetched in the background when missing or old).
    pub fn get() -> Option<Arc<Db>> {
        let path = table();
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let stale = modified.is_none_or(|m| m.elapsed().unwrap_or_default() > MAX_AGE);
        if stale && !FETCHING.swap(true, Ordering::SeqCst) {
            std::thread::spawn(|| {
                if let Err(e) = fetch() {
                    crate::config::log::error(format!("discord games: {e}"));
                }
                FETCHING.store(false, Ordering::SeqCst);
            });
        }
        let modified = modified?;
        let mut loaded = LOADED.lock().unwrap();
        if loaded.as_ref().is_none_or(|(at, _)| *at != modified) {
            *loaded = Some((modified, Arc::new(load(&std::fs::read_to_string(&path).ok()?))));
        }
        loaded.as_ref().map(|(_, d)| d.clone())
    }

    fn load(text: &str) -> Db {
        let mut db = Db { by_file: HashMap::new(), games: Vec::new() };
        let mut index: HashMap<(String, String), usize> = HashMap::new();
        for line in text.lines() {
            let mut f = line.split('\t');
            let (Some(exe), Some(name), Some(id), Some(icon)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
            let gi = *index.entry((id.to_string(), name.to_string())).or_insert_with(|| {
                db.games.push((name.to_string(), id.to_string(), icon.to_string()));
                db.games.len() - 1
            });
            let end = exe.trim_start_matches('>').to_string();
            let file = end.rsplit('/').next().unwrap_or(&end).to_string();
            db.by_file.entry(file).or_default().push((end, gi));
        }
        db
    }

    #[derive(serde::Deserialize)]
    struct App {
        id: String,
        name: String,
        #[serde(default)]
        icon_hash: Option<String>,
        #[serde(default)]
        executables: Option<Vec<Exe>>,
    }

    #[derive(serde::Deserialize)]
    struct Exe {
        name: String,
        os: String,
        #[serde(default)]
        is_launcher: bool,
    }

    /// Downloads the list and keeps only what we look up: this system's
    /// program names, the game's name, id and icon.
    fn fetch() -> Result<(), String> {
        let dir = crate::config::cache_dir();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("discord-games.json");
        crate::update::download(URL, &tmp, 0, |_| {})?;
        let apps: Vec<App> = serde_json::from_slice(&std::fs::read(&tmp).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&tmp);
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        let mut out = String::new();
        for a in &apps {
            for e in a.executables.iter().flatten().filter(|e| e.os == OS && !e.is_launcher) {
                let icon = a.icon_hash.as_deref().unwrap_or("");
                out.push_str(&format!("{}\t{}\t{}\t{}\n", clean(&e.name.to_lowercase()), clean(&a.name), clean(&a.id), clean(icon)));
            }
        }
        if out.is_empty() {
            return Err("empty list".into());
        }
        std::fs::write(table(), out).map_err(|e| e.to_string())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn matching() {
            let db = super::load(
                "dota2.exe\tDota 2\t356875988589740042\t6b4b\ngarrysmod/hl2.exe\tGarry's Mod\t1\tab\n>hl2.exe\tHalf-Life 2\t2\t\ngame.exe\tSomething\t3\tcd\n",
            );
            let g = db.find(r"D:\Steam\steamapps\common\dota 2 beta\game\bin\win64\dota2.exe").unwrap();
            assert_eq!(g.name, "Dota 2");
            assert_eq!(g.icon.as_deref(), Some("https://cdn.discordapp.com/app-icons/356875988589740042/6b4b.png?size=128"));
            assert_eq!(db.find(r"C:\Games\GarrysMod\hl2.exe").unwrap().name, "Garry's Mod");
            assert_eq!(db.find(r"C:\Games\HL2\hl2.exe").unwrap().name, "Half-Life 2");
            assert!(db.find(r"C:\x\game.exe").is_none());
            assert!(db.find(r"C:\x\chrome.exe").is_none());
            assert_eq!(db.icon_of("dota 2").as_deref(), g.icon.as_deref());
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn names() {
        use super::game_of;
        assert_eq!(game_of(r"D:\Steam\steamapps\common\dota 2 beta\game\bin\win64\dota2.exe").as_deref(), Some("Dota 2"));
        assert_eq!(game_of(r"C:\Games\steamapps\common\Hollow Knight\hollow_knight.exe").as_deref(), Some("Hollow Knight"));
        assert_eq!(game_of(r"C:\Games\steamapps\common\Hollow Knight\UnityCrashHandler64.exe"), None);
        assert_eq!(game_of(r"C:\Program Files (x86)\Steam\steamapps\common\Steamworks Shared\x.exe"), None);
        assert_eq!(game_of(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), None);
        assert_eq!(game_of(r"C:\Riot Games\Riot Client\RiotClientServices.exe"), None);
    }
}
