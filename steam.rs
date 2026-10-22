//! Host Steam session control. OS probes run off the HTTP thread.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
pub struct Status {
    pub active: bool,
    pub managed: bool,
    pub phase: String,
    pub running: Option<bool>,
    pub big_picture_open: Option<bool>,
    pub games: Vec<Game>,
    pub error: Option<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            active: false,
            managed: false,
            phase: "idle".into(),
            running: None,
            big_picture_open: None,
            games: vec![],
            error: None,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct Game {
    pub app_id: String,
    pub name: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Process {
    pid: u32,
    started: u64,
    started_local: String,
    exe: String,
    #[serde(default)]
    app_id: Option<String>,
}
#[derive(Deserialize)]
struct Probe {
    root: Option<PathBuf>,
    running: bool,
    big_picture_open: Option<bool>,
    processes: Vec<Process>,
}

enum Action {
    Open,
    Close,
}
pub struct Session {
    status: Arc<Mutex<Status>>,
    tx: mpsc::Sender<Action>,
}
impl Default for Session {
    fn default() -> Self {
        let status = Arc::new(Mutex::new(Status {
            phase: "idle".into(),
            ..Status::default()
        }));
        let shared = status.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut grace = Instant::now();
            let mut missing_since = None;
            let mut owned = false;
            loop {
                match rx.recv_timeout(Duration::from_millis(1000)) {
                    Ok(Action::Open) => {
                        match platform::probe().and_then(|p| {
                            require_available(&p)?;
                            platform::open()
                        }) {
                            Ok(()) => {
                                owned = true;
                                shared.lock().unwrap().managed = true;
                                grace = Instant::now() + Duration::from_secs(12);
                                missing_since = None;
                            }
                            Err(error) => {
                                let mut s = shared.lock().unwrap();
                                s.active = false;
                                s.phase = "error".into();
                                s.error = Some(error);
                                continue;
                            }
                        }
                    }
                    Ok(Action::Close) => {
                        if !owned {
                            continue;
                        }
                        let result = close_host();
                        let mut s = shared.lock().unwrap();
                        match result {
                            Ok(()) => {
                                owned = false;
                                s.active = false;
                                s.managed = false;
                                s.phase = "idle".into();
                                s.games.clear();
                                s.big_picture_open = Some(false);
                                s.error = None;
                            }
                            Err(error) => {
                                s.phase = "close_failed".into();
                                s.error = Some(error);
                            }
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if !shared.lock().unwrap().active {
                    continue;
                }
                match platform::probe() {
                    Ok(probe) => {
                        let games = detected_games(&probe);
                        let mut s = shared.lock().unwrap();
                        // A Close queued during this probe must remain visible as closing.
                        if s.phase == "closing" {
                            continue;
                        }
                        reconcile(
                            &mut s,
                            &probe,
                            games,
                            Instant::now(),
                            grace,
                            &mut missing_since,
                        );
                        if !s.active {
                            owned = false;
                        }
                    }
                    Err(error) => {
                        let mut s = shared.lock().unwrap();
                        if s.phase != "closing" {
                            s.phase = "unknown".into();
                            s.error = Some(error);
                        }
                    }
                }
            }
        });
        Self { status, tx }
    }
}

fn require_available(p: &Probe) -> Result<(), String> {
    if !p.running {
        return Err("Steam must already be running in the desktop session".into());
    }
    if p.big_picture_open != Some(false) {
        return Err("Big Picture is already open or cannot be detected; refusing to take over an unmanaged session".into());
    }
    if !detected_games(p).is_empty() {
        return Err(
            "a Steam game is already running; refusing to take over an unmanaged session".into(),
        );
    }
    Ok(())
}
impl Session {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }
    pub fn open(&self) -> Result<(), String> {
        let mut s = self.status.lock().unwrap();
        if s.phase == "closing" || s.phase == "close_failed" {
            return Err("finish closing the Steam session first".into());
        }
        if s.active {
            return Ok(());
        }
        self.tx
            .send(Action::Open)
            .map_err(|_| "Steam worker unavailable")?;
        *s = Status {
            active: true,
            phase: "starting".into(),
            ..Status::default()
        };
        Ok(())
    }
    pub fn close(&self) -> Result<bool, String> {
        let mut s = self.status.lock().unwrap();
        if !s.active {
            return Ok(false);
        }
        if s.phase != "closing" {
            self.tx
                .send(Action::Close)
                .map_err(|_| "Steam worker unavailable")?;
            s.phase = "closing".into();
            s.error = None;
        }
        Ok(true)
    }
}

fn reconcile(
    s: &mut Status,
    p: &Probe,
    games: Vec<Game>,
    now: Instant,
    grace: Instant,
    missing: &mut Option<Instant>,
) {
    s.running = Some(p.running);
    s.big_picture_open = p.big_picture_open;
    s.games = games;
    if !p.running {
        s.active = false;
        s.managed = false;
        s.phase = "idle".into();
        s.games.clear();
        s.error = None;
        return;
    }
    // Failed cleanup needs an explicit retry; do not erase its diagnostic.
    if s.phase == "close_failed" {
        return;
    }
    s.error = None;
    if !s.games.is_empty() {
        s.phase = "game".into();
        *missing = None;
    } else if p.big_picture_open == Some(true) {
        s.phase = "browsing".into();
        *missing = None;
    } else if p.big_picture_open.is_none() {
        s.phase = "unknown".into();
        *missing = None;
        s.error =
            Some("Big Picture window detection is unavailable in this desktop session".into());
    } else if now < grace {
        s.phase = "starting".into();
    } else {
        let since = *missing.get_or_insert(now);
        if now.duration_since(since) >= Duration::from_secs(3) {
            s.active = false;
            s.managed = false;
            s.phase = "idle".into();
        } else {
            s.phase = "transitioning".into();
        }
    }
}

fn log_processes(root: &Path) -> HashMap<u32, (String, String)> {
    let mut result = HashMap::new();
    let Ok(mut file) = fs::File::open(root.join("logs/gameprocess_log.txt")) else {
        return result;
    };
    // Bound startup work; never retain log command lines in public status.
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(8 * 1024 * 1024)));
    let mut data = String::new();
    let _ = file.read_to_string(&mut data);
    for line in data.lines() {
        let Some((_, tail)) = line.split_once("AppID ") else {
            continue;
        };
        let parts: Vec<_> = tail.split_whitespace().take(6).collect();
        if parts.len() >= 4 && parts[1] == "adding" && parts[2] == "PID" {
            if let Ok(pid) = parts[3].parse() {
                if let Some(stamp) = line.get(1..20) {
                    result.insert(pid, (parts[0].to_string(), stamp.to_string()));
                }
            }
        } else if parts.len() >= 6 && parts[1..5] == ["no", "longer", "tracking", "PID"] {
            if let Ok(pid) = parts[5].trim_end_matches(',').parse::<u32>() {
                result.remove(&pid);
            }
        }
    }
    result
}

// Small KeyValues tokenizer supporting comments, escaped paths and nested objects.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else if c == '"' {
            let mut token = String::new();
            while let Some(c) = chars.next() {
                if c == '"' {
                    break;
                }
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        token.push(next);
                    }
                } else {
                    token.push(c);
                }
            }
            out.push(token);
        } else if c == '{' || c == '}' {
            out.push(c.to_string());
        }
    }
    out
}
fn value(tokens: &[String], key: &str) -> Option<String> {
    tokens
        .windows(2)
        .find(|p| p[0].eq_ignore_ascii_case(key) && p[1] != "{")
        .map(|p| p[1].clone())
}
fn game_name(root: &Path, id: &str) -> Option<String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut libraries = vec![root.to_path_buf()];
    if let Ok(data) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        for pair in tokens(&data).windows(2) {
            if pair[0] == "path" {
                libraries.push(PathBuf::from(&pair[1]));
            }
        }
    }
    libraries
        .into_iter()
        .find_map(|p| {
            fs::read_to_string(p.join(format!("steamapps/appmanifest_{id}.acf")))
                .ok()
                .and_then(|data| value(&tokens(&data), "name"))
        })
        .or_else(|| shortcut_name(root, id))
}

fn shortcut_name(root: &Path, id: &str) -> Option<String> {
    let raw = id.parse::<u64>().ok()?;
    let shortcut_id = if raw > u32::MAX as u64 {
        (raw >> 32) as u32
    } else {
        raw as u32
    };
    for user in fs::read_dir(root.join("userdata")).ok()?.flatten() {
        let path = user.path().join("config/shortcuts.vdf");
        if fs::metadata(&path).is_ok_and(|m| m.len() > 4 * 1024 * 1024) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let mut at = 0;
        let Some(tree) = binary_vdf(&bytes, &mut at, 0) else {
            continue;
        };
        if let Some(entries) = tree.get("shortcuts").and_then(|v| v.as_object()) {
            for entry in entries.values() {
                if entry.get("appid").and_then(|v| v.as_u64()) == Some(shortcut_id as u64) {
                    if let Some(name) = entry.get("appname").and_then(|v| v.as_str()) {
                        return Some(name.into());
                    }
                }
            }
        }
    }
    None
}

fn binary_vdf(bytes: &[u8], at: &mut usize, depth: usize) -> Option<serde_json::Value> {
    if depth > 16 {
        return None;
    }
    let mut object = serde_json::Map::new();
    while *at < bytes.len() {
        let kind = bytes[*at];
        *at += 1;
        if kind == 8 {
            return Some(object.into());
        }
        let end = *at + bytes.get(*at..)?.iter().position(|b| *b == 0)?;
        let key = String::from_utf8_lossy(&bytes[*at..end]).to_lowercase();
        *at = end + 1;
        let value = match kind {
            0 => binary_vdf(bytes, at, depth + 1)?,
            1 => {
                let end = *at + bytes.get(*at..)?.iter().position(|b| *b == 0)?;
                let text = String::from_utf8_lossy(&bytes[*at..end]).to_string();
                *at = end + 1;
                text.into()
            }
            2 => {
                let n = u32::from_le_bytes(bytes.get(*at..*at + 4)?.try_into().ok()?);
                *at += 4;
                n.into()
            }
            _ => return None,
        };
        object.insert(key, value);
    }
    Some(object.into())
}
fn candidate_processes(p: &Probe) -> Vec<(Process, String)> {
    let logged = p.root.as_deref().map(log_processes).unwrap_or_default();
    p.processes
        .iter()
        .filter_map(|process| {
            let id = process.app_id.clone().or_else(|| {
                let (id, stamp) = logged.get(&process.pid)?;
                // Bind a log event to this process instance, including for shortcuts
                // outside steamapps. A reused PID must not identify/kill another app.
                let a = stamp_seconds(stamp)?;
                let b = stamp_seconds(&process.started_local)?;
                (a.abs_diff(b) <= 15).then(|| id.clone())
            })?;
            let name = Path::new(&process.exe)
                .file_name()?
                .to_string_lossy()
                .to_lowercase();
            if id == "0"
                || matches!(
                    name.as_str(),
                    "steam"
                        | "steam.exe"
                        | "steamwebhelper"
                        | "steamwebhelper.exe"
                        | "steamservice.exe"
                        | "wineserver"
                        | "services.exe"
                        | "explorer.exe"
                        | "rpcss.exe"
                        | "svchost.exe"
                        | "pressure-vessel-adverb"
                        | "steam-runtime-launcher-service"
                )
            {
                return None;
            }
            Some((process.clone(), id))
        })
        .collect()
}

fn stamp_seconds(stamp: &str) -> Option<i64> {
    let fields: Vec<i64> = stamp
        .split(['-', ' ', ':'])
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if fields.len() != 6 {
        return None;
    }
    let (year, month, day) = (fields[0], fields[1], fields[2]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + day - 1;
    Some(((days * 24 + fields[3]) * 60 + fields[4]) * 60 + fields[5])
}
fn detected_games(p: &Probe) -> Vec<Game> {
    let mut ids: Vec<_> = candidate_processes(p)
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| Game {
            name: p.root.as_deref().and_then(|root| game_name(root, &id)),
            app_id: id,
        })
        .collect()
}
fn close_host() -> Result<(), String> {
    let probe = platform::probe()?;
    let games = candidate_processes(&probe);
    for (process, _) in &games {
        platform::stop(process, false)?;
    }
    if !games.is_empty() {
        thread::sleep(Duration::from_secs(3));
    }
    for (process, _) in &games {
        platform::stop(process, true)?;
    }
    platform::close_picture()?;
    thread::sleep(Duration::from_millis(900));
    let final_probe = platform::probe()?;
    if !candidate_processes(&final_probe).is_empty() {
        return Err("a Steam game is still running; retry Close".into());
    }
    match final_probe.big_picture_open {
        Some(false) => platform::hide(),
        Some(true) => Err("Steam Big Picture is still open; retry Close".into()),
        None => Err("could not confirm Big Picture closed in this desktop session".into()),
    }
}

#[cfg(windows)]
#[path = "steam_windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "steam_linux.rs"]
mod platform;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_closures_and_game_transitions() {
        let now = Instant::now();
        let mut missing = None;
        let mut s = Status {
            active: true,
            ..Status::default()
        };
        let mut p = Probe {
            root: None,
            running: true,
            big_picture_open: Some(true),
            processes: vec![],
        };
        reconcile(&mut s, &p, vec![], now, now, &mut missing);
        assert_eq!(s.phase, "browsing");
        p.big_picture_open = Some(false);
        reconcile(
            &mut s,
            &p,
            vec![Game {
                app_id: "42".into(),
                name: None,
            }],
            now,
            now,
            &mut missing,
        );
        assert_eq!(s.phase, "game");
        reconcile(&mut s, &p, vec![], now, now, &mut missing);
        assert!(s.active);
        reconcile(
            &mut s,
            &p,
            vec![],
            now + Duration::from_secs(4),
            now,
            &mut missing,
        );
        assert!(!s.active);
        s.active = true;
        p.big_picture_open = None;
        reconcile(&mut s, &p, vec![], now, now, &mut missing);
        assert!(s.active);
        assert_eq!(s.phase, "unknown");
        p.running = false;
        reconcile(&mut s, &p, vec![], now, now, &mut missing);
        assert!(!s.active);
    }
    #[test]
    fn keyvalues_paths_and_titles() {
        let t = tokens("// comment\n\"path\" \"D:\\\\Games\" \"name\" \"A \\\"Game\\\"\"");
        assert_eq!(value(&t, "path").as_deref(), Some("D:\\Games"));
        assert_eq!(value(&t, "name").as_deref(), Some("A \"Game\""));
    }
    #[test]
    fn unmanaged_close_never_sends_a_host_command() {
        let (tx, rx) = mpsc::channel();
        let session = Session {
            status: Arc::new(Mutex::new(Status::default())),
            tx,
        };
        assert!(!session.close().unwrap());
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn preflight_refuses_existing_or_unobservable_sessions() {
        let mut p = Probe {
            root: None,
            running: true,
            big_picture_open: Some(true),
            processes: vec![],
        };
        assert!(require_available(&p).is_err());
        p.big_picture_open = None;
        assert!(require_available(&p).is_err());
        p.big_picture_open = Some(false);
        assert!(require_available(&p).is_ok());
        p.processes.push(Process {
            pid: 42,
            started: 1,
            started_local: String::new(),
            exe: "game".into(),
            app_id: Some("42".into()),
        });
        assert!(require_available(&p).is_err());
        p.processes.clear();
        p.running = false;
        assert!(require_available(&p).is_err());
    }
    #[test]
    fn general_close_closes_video_and_steam_while_steam_close_is_targeted() {
        let (tx, rx) = mpsc::channel();
        let session = Session {
            status: Arc::new(Mutex::new(Status {
                active: true,
                managed: true,
                phase: "browsing".into(),
                ..Status::default()
            })),
            tx,
        };
        let mut state = crate::State {
            steam: Some(session),
            ..crate::State::default()
        };
        let item = || {
            serde_json::from_value(
                serde_json::json!({"url":"https://example.com/video.mp4","local":true}),
            )
            .unwrap()
        };
        state.current = Some(item());
        let cfg = crate::Config {
            bind: "127.0.0.1:8765".into(),
            moonlight: "moonlight".into(),
            moonlight_args: vec![],
            ytdlp_cookies_from_browser: None,
            cookies_browser: "off".into(),
            cookies_profile: None,
            remote_origin_servers: vec![],
            default_screen: None,
            default_quality: Some("720".into()),
            default_video_codec: "h264".into(),
            subtitles: false,
            preferred_language: None, track_preferences: Default::default(),
        };
        let request = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/close");
        crate::handle(request.into(), &cfg, &mut state);
        assert!(state.current.is_none());
        assert!(matches!(rx.try_recv(), Ok(Action::Close)));
        assert!(state.steam.as_ref().unwrap().status().managed);
        state.steam.as_ref().unwrap().status.lock().unwrap().phase = "browsing".into();
        state.current = Some(item());
        let request = tiny_http::TestRequest::new()
            .with_method(tiny_http::Method::Post)
            .with_path("/steam/close");
        crate::handle(request.into(), &cfg, &mut state);
        assert!(state.current.is_some());
        assert!(matches!(rx.try_recv(), Ok(Action::Close)));
        state.steam.as_ref().unwrap().status.lock().unwrap().phase = "browsing".into();
        state.close_deadline = Some(std::time::Instant::now());
        assert!(crate::expire_close_timer(&mut state, std::time::Instant::now()));
        assert!(state.current.is_none());
        assert!(matches!(rx.try_recv(), Ok(Action::Close)));
    }
    #[test]
    fn game_identity_rejects_stale_log_pid_but_accepts_linux_markers() {
        let root =
            std::env::temp_dir().join(format!("mediabridge-steam-test-{}", std::process::id()));
        fs::create_dir_all(root.join("logs")).unwrap();
        fs::write(root.join("logs/gameprocess_log.txt"), "[2026-10-21 12:00:00] AppID 42 adding PID 123 as a tracked process\n[2026-10-21 12:00:00] AppID 43 adding PID 124 as a tracked process\n[2026-10-21 12:00:01] AppID 43 no longer tracking PID 124, exit code 0\n").unwrap();
        let mut p = Probe {
            root: Some(root.clone()),
            running: true,
            big_picture_open: Some(false),
            processes: vec![Process {
                pid: 123,
                started: 1,
                started_local: "2026-10-21 12:00:00".into(),
                exe: "game.exe".into(),
                app_id: None,
            }],
        };
        assert_eq!(detected_games(&p)[0].app_id, "42");
        p.processes[0].started_local = "2026-10-21 13:00:00".into();
        assert!(detected_games(&p).is_empty());
        p.processes[0].app_id = Some("12345678901234567890".into());
        assert_eq!(detected_games(&p)[0].app_id, "12345678901234567890");
        assert!(!log_processes(&root).contains_key(&124));
        fs::remove_file(root.join("logs/gameprocess_log.txt")).unwrap();
        fs::remove_dir(root.join("logs")).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn binary_shortcut_metadata_preserves_unsigned_ids() {
        let mut bytes = b"\x00shortcuts\x00\x000\x00\x02appid\x00".to_vec();
        bytes.extend_from_slice(&0xf1234567u32.to_le_bytes());
        bytes.extend_from_slice(b"\x01AppName\x00My Game\x00\x08\x08\x08");
        let tree = binary_vdf(&bytes, &mut 0, 0).unwrap();
        assert_eq!(tree["shortcuts"]["0"]["appid"].as_u64(), Some(0xf1234567));
        assert_eq!(tree["shortcuts"]["0"]["appname"], "My Game");
    }
    #[test]
    #[ignore = "read-only probe of the current desktop; run explicitly"]
    fn host_probe_read_only() {
        let probe = platform::probe().unwrap();
        std::println!(
            "Steam running: {}; Big Picture: {:?}; detected games: {}",
            probe.running,
            probe.big_picture_open,
            detected_games(&probe).len()
        );
    }
}
