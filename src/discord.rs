//! Discord Rich Presence: "Clipping with blinker" on the user's profile,
//! with the logo and a button to blinker.cool. Spoken to the running
//! Discord app over its local IPC (a Unix socket, a named pipe on Windows):
//! frames of (opcode, length, JSON). Off unless `discord_presence` is set.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

/// The Discord application blinker shows up as (discord.com/developers).
const CLIENT_ID: &str = "1562442008918888531";

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;

#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Pipe = std::fs::File;

/// Discord's socket / pipe, if Discord runs.
fn connect() -> Option<Pipe> {
    #[cfg(unix)]
    {
        let mut dirs: Vec<std::path::PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .iter()
            .filter_map(|k| std::env::var_os(k))
            .map(std::path::PathBuf::from)
            .collect();
        dirs.push("/tmp".into());
        for d in dirs {
            for i in 0..10 {
                if let Ok(s) = Pipe::connect(d.join(format!("discord-ipc-{i}"))) {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                    return Some(s);
                }
            }
        }
        None
    }
    #[cfg(windows)]
    {
        (0..10).find_map(|i| std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")).ok())
    }
}

fn send(p: &mut Pipe, op: u32, json: &serde_json::Value) -> std::io::Result<()> {
    let body = json.to_string();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body.as_bytes());
    p.write_all(&frame)
}

/// One frame back: (opcode, JSON).
fn recv(p: &mut Pipe) -> std::io::Result<(u32, serde_json::Value)> {
    let mut head = [0u8; 8];
    p.read_exact(&mut head)?;
    let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut body = vec![0u8; len];
    p.read_exact(&mut body)?;
    Ok((op, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)))
}

/// Screenshots and clips saved today: (shots, clips).
/// Today's screenshots and clips, counted as they are made (saved to disk
/// or only copied): `day shots clips` in the cache folder, so a restart
/// keeps them.
fn counts_file() -> std::path::PathBuf {
    crate::config::cache_dir().join("today.txt")
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn today_counts() -> (usize, usize) {
    let text = std::fs::read_to_string(counts_file()).unwrap_or_default();
    let mut f = text.split_whitespace();
    match (f.next(), f.next().and_then(|n| n.parse().ok()), f.next().and_then(|n| n.parse().ok())) {
        (Some(day), Some(shots), Some(clips)) if day == today() => (shots, clips),
        _ => (0, 0),
    }
}

/// One more screenshot (`clip` false) or clip (a recording, a GIF, a replay).
pub fn count(clip: bool) {
    let (mut shots, mut clips) = today_counts();
    if clip {
        clips += 1;
    } else {
        shots += 1;
    }
    let _ = std::fs::create_dir_all(crate::config::cache_dir());
    let _ = std::fs::write(counts_file(), format!("{} {shots} {clips}", today()));
}

/// blinker's Discord server.
pub const INVITE: &str = "Dy4pUDEV8x";

/// The invite: straight in the Discord app when it's installed, else in
/// the browser.
pub fn open_invite() {
    if crate::win::url_scheme_registered("discord") {
        crate::win::open_url(&format!("discord://-/invite/{INVITE}"));
    } else {
        crate::win::open_url(&format!("https://discord.gg/{INVITE}"));
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What the profile shows: while recording, what and for how long (a red
/// badge on the logo); otherwise today's count under "Ready to clip" when
/// the instant replay runs, or under the tagline.
fn activity(shared: &crate::runtime::Shared) -> serde_json::Value {
    let mut a = serde_json::json!({
        "assets": {
            "large_image": "https://blinker.cool/assets/discord-logo.png",
            "large_text": "hewwo everynyan ;3",
        },
        "buttons": [{ "label": "blinker.cool", "url": "https://blinker.cool" }],
    });
    let note = shared.recording_note.lock().unwrap().clone();
    // a game in front (or running): "Clipping Dota 2"
    let game = shared.config().discord_game.then(crate::games::running).flatten();
    match note {
        Some((start, what)) => {
            a["details"] = game.as_ref().map_or("Recording the screen".to_string(), |g| format!("Recording {}", g.name)).into();
            a["state"] = what.into();
            a["timestamps"] = serde_json::json!({ "start": start });
            a["assets"]["small_image"] = "https://blinker.cool/assets/discord-rec.png".into();
            a["assets"]["small_text"] = "recording".into();
        }
        None => {
            let cfg = shared.config();
            let (shots, clips) = today_counts();
            let tagline = "Screenshots & Clipping";
            let replay = shared.replay_live.load(std::sync::atomic::Ordering::SeqCst);
            let stats = (cfg.discord_stats && shots + clips > 0)
                .then(|| format!("{} · {} today", plural(shots, "screenshot", "screenshots"), plural(clips, "clip", "clips")));
            match (replay, stats) {
                (true, Some(s)) => {
                    a["details"] = "Ready to clip".into();
                    a["state"] = s.into();
                }
                (true, None) => {
                    a["details"] = "Ready to clip".into();
                    a["state"] = tagline.into();
                }
                (false, Some(s)) => {
                    a["details"] = tagline.into();
                    a["state"] = s.into();
                }
                (false, None) => a["details"] = tagline.into(),
            }
            if let Some(g) = game {
                if a["state"].is_null() {
                    a["state"] = a["details"].clone();
                }
                a["details"] = format!("Clipping {}", g.name).into();
                // the game's icon in the corner of blinker's logo
                if let Some(icon) = g.icon {
                    a["assets"]["small_image"] = icon.into();
                    a["assets"]["small_text"] = g.name.into();
                }
            }
        }
    }
    a
}

fn set_activity(p: &mut Pipe, activity: Option<serde_json::Value>, nonce: &mut u64) -> std::io::Result<()> {
    *nonce += 1;
    let mut args = serde_json::json!({ "pid": std::process::id() });
    if let Some(a) = activity {
        args["activity"] = a;
    }
    send(p, OP_FRAME, &serde_json::json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce.to_string() }))?;
    let (_, answer) = recv(p)?;
    if answer.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
        return Err(std::io::Error::other(answer.pointer("/data/message").and_then(|m| m.as_str()).unwrap_or("error").to_string()));
    }
    Ok(())
}

/// In the tray process: keeps the presence in step with the setting, with
/// what blinker does (a recording, today's count) and with Discord starting
/// and quitting, while blinker runs.
pub fn run(shared: Arc<crate::runtime::Shared>) {
    std::thread::spawn(move || {
        let mut pipe: Option<Pipe> = None;
        let mut nonce = 0u64;
        let mut shown = serde_json::Value::Null;
        let mut since = std::time::Instant::now();
        loop {
            let on = shared.config().discord_presence;
            match (&mut pipe, on) {
                (None, true) => {
                    if let Some(mut p) = connect() {
                        let hello = serde_json::json!({ "v": 1, "client_id": CLIENT_ID });
                        let now = activity(&shared);
                        let ok = send(&mut p, OP_HANDSHAKE, &hello)
                            .and_then(|_| recv(&mut p))
                            .and_then(|(op, _)| if op == OP_FRAME { Ok(()) } else { Err(std::io::Error::other("discord said no")) })
                            .and_then(|_| set_activity(&mut p, Some(now.clone()), &mut nonce));
                        match ok {
                            Ok(()) => {
                                pipe = Some(p);
                                shown = now;
                                since = std::time::Instant::now();
                            }
                            Err(e) => crate::config::log::error(format!("discord: {e}")),
                        }
                    }
                }
                (Some(p), false) => {
                    let _ = set_activity(p, None, &mut nonce);
                    pipe = None;
                    shown = serde_json::Value::Null;
                }
                (Some(p), true) => {
                    // a change shows at once; once a minute it's sent anyway
                    // (Discord may have quit or restarted)
                    let now = activity(&shared);
                    if now != shown || since.elapsed() > Duration::from_secs(60) {
                        since = std::time::Instant::now();
                        if set_activity(p, Some(now.clone()), &mut nonce).is_ok() {
                            shown = now;
                        } else {
                            pipe = None;
                        }
                    }
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}
