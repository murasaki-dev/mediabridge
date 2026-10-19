//! A small, single-file media bridge. Browser and Moonlight are separate
//! programs, with the installers optionally bundling their runtime files.

#![cfg_attr(windows, windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{HashSet, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Read};
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(windows)]
use std::sync::Mutex;
use std::sync::{OnceLock, RwLock};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

#[cfg(unix)]
unsafe extern "C" { fn kill(pid: i32, signal: i32) -> i32; }

#[cfg(windows)]
macro_rules! println {
    ($($arg:tt)*) => {{ crate::append_windows_log(format!($($arg)*)); }};
}
#[cfg(windows)]
macro_rules! eprintln {
    ($($arg:tt)*) => {{ crate::append_windows_log(format!($($arg)*)); }};
}

#[cfg(target_os = "linux")]
macro_rules! println {
    ($($arg:tt)*) => {{ crate::append_linux_log(format!($($arg)*)); }};
}
#[cfg(target_os = "linux")]
macro_rules! eprintln {
    ($($arg:tt)*) => {{ crate::append_linux_log(format!($($arg)*)); }};
}

#[cfg(windows)]
fn windows_log_path() -> PathBuf {
    env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(env::temp_dir)
        .join("MediaBridge").join("mediabridge.log")
}

#[cfg(windows)]
fn append_windows_log(message: String) {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    let path = windows_log_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs()).unwrap_or_default();
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
}

#[cfg(windows)]
fn initialize_windows_log() {
    use std::fs::{self, OpenOptions};
    let path = windows_log_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    let _ = OpenOptions::new().create(true).append(true).open(path);
}

#[cfg(target_os = "linux")]
fn linux_log_path() -> PathBuf {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(env::temp_dir)
        .join("mediabridge")
        .join("mediabridge.log")
}

#[cfg(target_os = "linux")]
fn append_linux_log(message: String) {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    let path = linux_log_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs()).unwrap_or_default();
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
}

#[cfg(target_os = "linux")]
fn initialize_linux_log() {
    use std::fs::{self, OpenOptions};
    let path = linux_log_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    let _ = OpenOptions::new().create(true).append(true).open(path);
}

#[derive(Clone)]
struct Config {
    bind: String,
    moonlight: String,
    moonlight_args: Vec<String>,
    ytdlp_cookies_from_browser: Option<String>,
    cookies_browser: String,
    cookies_profile: Option<String>,
    remote_origin_servers: Vec<String>,
    default_screen: Option<u8>,
    default_quality: Option<String>,
    subtitles: bool,
    preferred_language: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct UserConfig {
    bind: String,
    remote_origin_servers: Vec<String>,
    default_screen: Option<u8>,
    default_quality: Option<String>,
    subtitles: bool,
    preferred_language: Option<String>,
    cookies_browser: String,
    cookies_profile: Option<String>,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8765".into(),
            remote_origin_servers: Vec::new(),
            default_screen: None,
            default_quality: None,
            subtitles: false,
            preferred_language: None,
            cookies_browser: if cfg!(target_os = "linux") || cfg!(windows) { "librewolf" } else { "auto" }.into(),
            cookies_profile: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct PlaybackPreferences {
    default_screen: Option<u8>,
    default_quality: Option<String>,
    subtitles: bool,
    preferred_language: Option<String>,
    cookies_browser: String,
    cookies_profile: Option<String>,
}

static REMOTE_ORIGINS: OnceLock<RwLock<Vec<String>>> = OnceLock::new();
static PLAYBACK_PREFERENCES: OnceLock<RwLock<PlaybackPreferences>> = OnceLock::new();

fn user_config_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or("could not locate the user's configuration directory")?;
        return Ok(config_home.join("mediabridge").join("mediabridge.json"));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let executable = env::current_exe()
            .map_err(|e| format!("could not locate MediaBridge executable: {e}"))?;
        let directory = executable
            .parent()
            .ok_or("MediaBridge executable has no parent directory")?;
        Ok(directory.join("mediabridge.json"))
    }
}

fn load_user_config() -> Result<UserConfig, String> {
    let path = user_config_path()?;
    if !path.exists() {
        #[cfg(target_os = "linux")]
        {
            // Migrate settings created by the previous executable-directory layout.
            let legacy = env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("mediabridge.json")));
            if let Some(legacy) = legacy.filter(|legacy| legacy.is_file()) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!("could not create configuration directory {}: {e}", parent.display())
                    })?;
                }
                std::fs::copy(&legacy, &path).map_err(|e| {
                    format!("could not migrate configuration from {}: {e}", legacy.display())
                })?;
                println!("Migrated configuration from {} to {}", legacy.display(), path.display());
            }
        }
    }
    if !path.exists() {
        let defaults = UserConfig::default();
        let contents = serde_json::to_string_pretty(&defaults).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("could not create configuration directory {}: {e}", parent.display())
            })?;
        }
        match std::fs::write(&path, format!("{contents}\n")) {
            Ok(()) => println!("Created configuration file at {}", path.display()),
            Err(error) => eprintln!("Could not create configuration file at {}: {error}", path.display()),
        }
        return Ok(defaults);
    }
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read configuration file {}: {e}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|e| format!("invalid configuration file {}: {e}", path.display()))
}

fn normalize_remote_origins(origins: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for raw in origins {
        let origin = raw.trim().trim_end_matches('/').to_string();
        if origin.is_empty() { continue; }
        let authority = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"));
        if authority.map_or(true, |host| host.is_empty() || host.contains('/') || host.contains('?') || host.contains('#') || host.contains('@') || host.chars().any(char::is_whitespace)) {
            return Err(format!("invalid web origin '{origin}'; use an origin such as https://party.example with no path"));
        }
        if !normalized.contains(&origin) { normalized.push(origin); }
    }
    Ok(normalized)
}

fn remote_origin_allowed(origin: &str) -> bool {
    REMOTE_ORIGINS.get().and_then(|list| list.read().ok())
        .is_some_and(|list| list.iter().any(|allowed| allowed == origin))
}

fn save_remote_origins(origins: Vec<String>) -> Result<Vec<String>, String> {
    let origins = normalize_remote_origins(origins)?;
    let path = user_config_path()?;
    let mut config = load_user_config()?;
    config.remote_origin_servers = origins.clone();
    let contents = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("could not create configuration directory {}: {e}", parent.display())
        })?;
    }
    std::fs::write(&path, format!("{contents}\n"))
        .map_err(|e| format!("could not save configuration file {}: {e}", path.display()))?;
    if let Some(list) = REMOTE_ORIGINS.get() {
        *list.write().map_err(|_| "remote origin settings are unavailable".to_string())? = origins.clone();
    }
    Ok(origins)
}

fn validate_preferences(preferences: &PlaybackPreferences) -> Result<(), String> {
    if preferences.default_screen.is_some_and(|screen| screen > 32) {
        return Err("preferred monitor must be between 0 and 32".into());
    }
    if preferences.default_quality.as_deref().is_some_and(|quality| !quality.eq_ignore_ascii_case("best") && quality.parse::<u16>().ok().map_or(true, |h| !(144..=4320).contains(&h))) {
        return Err("default resolution must be 'best' or a height from 144 to 4320".into());
    }
    if preferences.preferred_language.as_deref().is_some_and(|language| language.contains(',') || language.contains('\n') || language.contains('\r')) {
        return Err("preferred subtitle language must not contain commas or line breaks".into());
    }
    if !matches!(preferences.cookies_browser.as_str(), "auto" | "off" | "librewolf" | "chrome" | "chromium" | "edge" | "firefox" | "brave" | "opera" | "vivaldi" | "whale") {
        return Err("cookie browser must be auto, off, or a browser supported by yt-dlp".into());
    }
    if preferences.cookies_profile.as_deref().is_some_and(|profile| profile.contains(',') || profile.contains('\n') || profile.contains('\r')) {
        return Err("cookie browser profile must not contain commas or line breaks".into());
    }
    Ok(())
}

fn default_cookie_browser() -> String { "auto".into() }

fn save_playback_preferences(preferences: PlaybackPreferences) -> Result<(), String> {
    validate_preferences(&preferences)?;
    let path = user_config_path()?;
    let mut config = load_user_config()?;
    config.default_screen = preferences.default_screen;
    config.default_quality = preferences.default_quality.clone();
    config.subtitles = preferences.subtitles;
    config.preferred_language = preferences.preferred_language.clone();
    config.cookies_browser = preferences.cookies_browser.clone();
    config.cookies_profile = preferences.cookies_profile.clone();
    let contents = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("could not create configuration directory {}: {e}", parent.display())
        })?;
    }
    std::fs::write(&path, format!("{contents}\n"))
        .map_err(|e| format!("could not save configuration file {}: {e}", path.display()))?;
    if let Some(live) = PLAYBACK_PREFERENCES.get() {
        *live.write().map_err(|_| "playback preferences are unavailable".to_string())? = preferences;
    }
    Ok(())
}

fn save_bind_address(bind: &str) -> Result<(), String> {
    let path = user_config_path()?;
    let mut config = load_user_config()?;
    config.bind = bind.to_string();
    let contents = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!("could not create configuration directory {}: {e}", parent.display())
        })?;
    }
    std::fs::write(&path, format!("{contents}\n"))
        .map_err(|e| format!("could not save configuration file {}: {e}", path.display()))
}

fn current_playback_preferences(cfg: &Config) -> PlaybackPreferences {
    let (cookies_browser, cookies_profile) = configured_cookie_source(cfg);
    PLAYBACK_PREFERENCES.get().and_then(|value| value.read().ok())
        .map(|value| value.clone()).unwrap_or_else(|| PlaybackPreferences {
            default_screen: cfg.default_screen,
            default_quality: cfg.default_quality.clone(),
            subtitles: cfg.subtitles,
            preferred_language: cfg.preferred_language.clone(),
            cookies_browser,
            cookies_profile,
        })
}

#[derive(Clone, Deserialize)]
struct QueueItem {
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    thumbnail_url: Option<String>,
    #[serde(default)]
    channel_avatar_url: Option<String>,
    #[serde(default)]
    channel_banner_url: Option<String>,
    #[serde(default, rename = "local")]
    local_playback: bool,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    screen: Option<u8>,
    #[serde(default)]
    subtitles: Option<bool>,
    #[serde(default)]
    browser: Option<String>,
    #[serde(default)]
    browser_args: Vec<String>,
    #[serde(default)]
    youtube_guest: bool,
    #[serde(default)]
    twitch_theater: bool,
    #[serde(default = "default_youtube_mode")]
    youtube_mode: String,
    #[serde(default)]
    autoplay: Option<bool>,
    #[serde(default)]
    autoplay_minutes: Option<u64>,
    #[serde(skip)]
    autoplay_generated: bool,
}

#[derive(Default)]
struct State {
    player: Option<Child>,
    mpv_socket: Option<PathBuf>,
    current: Option<QueueItem>,
    queue: VecDeque<QueueItem>,
    history: VecDeque<QueueItem>,
    mirror: Option<Child>,
    moonlight_pairing: Option<MoonlightPairing>,
    moonlight_pair_result: Option<(String, bool)>,
    player_error_tx: Option<Sender<(u64, String)>>,
    player_error_rx: Option<Receiver<(u64, String)>>,
    metadata_tx: Option<Sender<(String, Result<serde_json::Value, String>)>>,
    metadata_rx: Option<Receiver<(String, Result<serde_json::Value, String>)>>,
    metadata_cancel: Option<Arc<AtomicBool>>,
    metadata_in_flight_url: Option<String>,
    pending_metadata: VecDeque<String>,
    pending_queue_metadata: VecDeque<String>,
    metadata_in_flight: bool,
    playback_epoch: u64,
    autoplay_enabled: bool,
    autoplay_until: Option<std::time::Instant>,
    autoplay_tx: Option<Sender<(String, Result<Vec<String>, String>)>>,
    autoplay_rx: Option<Receiver<(String, Result<Vec<String>, String>)>>,
    autoplay_in_flight: Option<String>,
    autoplay_cancel: Option<Arc<AtomicBool>>,
    autoplay_requested_seed: Option<String>,
    autoplay_seen_ids: HashSet<String>,
    autoplay_waiting_to_advance: bool,
    last_mpv_volume: Option<f64>,
    playback_error: Option<String>,
    restart_bind: Option<String>,
    #[cfg(any(windows, target_os = "linux"))]
    librewolf_bidi: Option<LibreWolfBidi>,
    #[cfg(any(windows, target_os = "linux"))]
    librewolf_bidi_retry_at: Option<std::time::Instant>,
}

struct MoonlightPairing {
    host: String,
    executable: String,
    child: Child,
    verifying: bool,
}

#[cfg(any(windows, target_os = "linux"))]
struct LibreWolfBidi {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    context_id: Option<String>,
    cached_at: Option<std::time::Instant>,
    cached_video: Option<LibreWolfVideoState>,
    autoplay_attempted: bool,
    fullscreen_attempted_at: Option<std::time::Instant>,
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Clone, Default)]
struct LibreWolfVideoState {
    title: Option<String>,
    current_time: Option<f64>,
    duration: Option<f64>,
    volume: Option<f64>,
    paused: Option<bool>,
    ended: bool,
}

#[derive(Deserialize)]
struct MirrorRequest {
    host: String,
    #[serde(default = "default_app")]
    app: String,
}

#[derive(Deserialize)]
struct SeekRequest { position: f64 }

#[derive(Deserialize)]
struct LinkRequest { url: String }

#[derive(Deserialize)]
struct QueueIndexRequest { index: usize, #[serde(default)] to: Option<usize> }

#[derive(Deserialize)]
struct AutoplayRequest {
    enabled: bool,
    #[serde(default)] duration_minutes: Option<u64>,
}

#[derive(Deserialize)]
struct VolumeRequest { volume: f64 }

#[derive(Deserialize)]
struct RemoteOriginsRequest { remote_origin_servers: Vec<String> }

#[derive(Deserialize)]
struct CookieCheckRequest {
    #[serde(default = "default_cookie_browser")]
    cookies_browser: String,
    #[serde(default)]
    cookies_profile: Option<String>,
    #[serde(default = "default_cookie_site")]
    site: String,
}

fn default_cookie_site() -> String { "youtube".into() }

#[derive(Deserialize)]
struct PlaybackPreferencesRequest {
    #[serde(default)]
    default_screen: Option<u8>,
    #[serde(default)]
    default_quality: Option<String>,
    #[serde(default)]
    subtitles: bool,
    #[serde(default)]
    preferred_language: Option<String>,
    #[serde(default = "default_cookie_browser")]
    cookies_browser: String,
    #[serde(default)]
    cookies_profile: Option<String>,
}

#[derive(Deserialize)]
struct PortChangeRequest { port: u16 }

impl From<PlaybackPreferencesRequest> for PlaybackPreferences {
    fn from(value: PlaybackPreferencesRequest) -> Self {
        Self { default_screen: value.default_screen, default_quality: value.default_quality,
            subtitles: value.subtitles, preferred_language: value.preferred_language,
            cookies_browser: value.cookies_browser, cookies_profile: value.cookies_profile }
    }
}

#[derive(Deserialize)]
struct MouseRequest {
    #[serde(default)] x: i32,
    #[serde(default)] y: i32,
    #[serde(default)] dx: i32,
    #[serde(default)] dy: i32,
    action: String,
    #[serde(default = "default_mouse_button")]
    button: String,
}

fn default_mouse_button() -> String { "left".into() }
fn default_youtube_mode() -> String { "regular".into() }

fn wayland_session() -> bool {
    match env::var("XDG_SESSION_TYPE").ok().as_deref() {
        Some("wayland") => true,
        Some("x11") => false,
        _ => env::var_os("WAYLAND_DISPLAY").is_some(),
    }
}

fn default_app() -> String { "Desktop".into() }

#[derive(Serialize)]
struct Status {
    current: Option<String>,
    queued: Vec<String>,
    queue_items: Vec<QueueItemStatus>,
    playing: bool,
    mirroring: bool,
    playback_state: String,
    playback_error: Option<String>,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    artwork_url: Option<String>,
    channel_avatar_url: Option<String>,
    channel_banner_url: Option<String>,
    media_url: Option<String>,
    position_seconds: Option<f64>,
    duration_seconds: Option<f64>,
    volume: Option<f64>,
    can_seek: bool,
    previous_available: bool,
    autoplay_enabled: bool,
    autoplay_until_epoch_ms: Option<u64>,
}

#[derive(Serialize)]
struct QueueItemStatus {
    url: String,
    title: Option<String>,
    uploader: Option<String>,
    thumbnail_url: Option<String>,
    channel_avatar_url: Option<String>,
    channel_banner_url: Option<String>,
    local: bool,
    quality: Option<String>,
    screen: Option<u8>,
    autoplay_generated: bool,
}

fn usage() -> &'static str {
    "mediabridge [--bind IP:PORT] [--moonlight PATH] [--moonlight-arg ARG]... [--cookies-from-browser BROWSER]\n\
     Choose browser and optional browser_args per /play or /queue request.\n\
     If browser is omitted or unavailable, MediaBridge uses the OS default browser.\n\
     --cookies-from-browser applies yt-dlp browser cookies to local mpv playback only.\n\
     Moonlight arguments may contain {host} and {app}; defaults are `stream {host} {app}`."
}

fn bundled_moonlight_path() -> Option<String> {
    let executable = env::current_exe().ok()?;
    let install_dir = executable.parent()?;
    #[cfg(windows)]
    let candidates = [install_dir.join("moonlight").join("Moonlight.exe")];
    #[cfg(target_os = "linux")]
    let candidates = [
        install_dir.join("moonlight-current").join("squashfs-root").join("AppRun"),
        install_dir.join("moonlight").join("AppRun"),
    ];
    #[cfg(not(any(windows, target_os = "linux")))]
    let candidates: [PathBuf; 0] = [];
    candidates.into_iter().find(|path| path.is_file()).map(|path| path.to_string_lossy().into_owned())
}

fn moonlight_command(cfg: &Config) -> Command {
    let mut command = Command::new(&cfg.moonlight);
    if let Some(directory) = Path::new(&cfg.moonlight).parent().filter(|path| !path.as_os_str().is_empty()) {
        command.current_dir(directory);
    }
    command
}

fn moonlight_pair_status(state: &mut State) -> serde_json::Value {
    if let Some(mut pairing) = state.moonlight_pairing.take() {
        match pairing.child.try_wait() {
            Ok(None) => {
                let response = if pairing.verifying { "Verifying the pairing" } else { "Waiting for the Sunshine PIN confirmation" };
                let result = serde_json::json!({"state":"waiting", "host":pairing.host, "message":response});
                state.moonlight_pairing = Some(pairing);
                return result;
            }
            Err(error) => {
                let host = pairing.host.clone();
                state.moonlight_pair_result = Some((host.clone(), false));
                return serde_json::json!({"state":"failed", "host":host, "error":format!("could not check Moonlight pairing: {error}")});
            }
            Ok(Some(status)) if !pairing.verifying && status.success() => {
                let mut command = Command::new(&pairing.executable);
                command.arg("list").arg(&pairing.host)
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                if let Some(directory) = Path::new(&pairing.executable).parent().filter(|path| !path.as_os_str().is_empty()) {
                    command.current_dir(directory);
                }
                match command.spawn() {
                    Ok(child) => {
                        pairing.child = child;
                        pairing.verifying = true;
                        state.moonlight_pairing = Some(pairing);
                        return serde_json::json!({"state":"waiting", "host":state.moonlight_pairing.as_ref().unwrap().host, "message":"Verifying the pairing"});
                    }
                    Err(error) => {
                        let host = pairing.host.clone();
                        state.moonlight_pair_result = Some((host.clone(), false));
                        return serde_json::json!({"state":"failed", "host":host, "error":format!("Moonlight could not verify the new pairing: {error}")});
                    }
                }
            }
            Ok(Some(status)) => {
                let host = pairing.host.clone();
                let paired = pairing.verifying && status.success();
                state.moonlight_pair_result = Some((host.clone(), paired));
                return if paired {
                    serde_json::json!({"state":"paired", "host":host})
                } else {
                    serde_json::json!({"state":"failed", "host":host, "error":"Moonlight pairing was not confirmed. Check the PIN in Moonlight and the Sunshine pairing page."})
                };
            }
        }
    }
    if let Some((host, paired)) = state.moonlight_pair_result.as_ref() {
        if *paired { serde_json::json!({"state":"paired", "host":host}) }
        else { serde_json::json!({"state":"failed", "host":host, "error":"Moonlight pairing did not complete. Check the PIN in Moonlight and the Sunshine pairing page."}) }
    } else {
        serde_json::json!({"state":"idle"})
    }
}

fn config() -> Result<Config, String> {
    let user = load_user_config()?;
    let mut bind = user.bind.clone();
    let remote_origin_servers = normalize_remote_origins(user.remote_origin_servers.clone())?;
    if user.default_screen.is_some_and(|screen| screen > 32) {
        return Err("configuration default_screen must be between 0 and 32".into());
    }
    if user.default_quality.as_deref().is_some_and(|quality| !quality.eq_ignore_ascii_case("best") && quality.parse::<u16>().ok().map_or(true, |h| !(144..=4320).contains(&h))) {
        return Err("configuration default_quality must be 'best' or a height from 144 to 4320".into());
    }
    if user.preferred_language.as_deref().is_some_and(|language| language.contains(',') || language.contains('\n') || language.contains('\r')) {
        return Err("preferred_language must not contain commas or line breaks".into());
    }
    if !matches!(user.cookies_browser.as_str(), "auto" | "off" | "librewolf" | "chrome" | "chromium" | "edge" | "firefox" | "brave" | "opera" | "vivaldi" | "whale") {
        return Err("configuration cookies_browser must be auto, off, or a browser supported by yt-dlp".into());
    }
    if user.cookies_profile.as_deref().is_some_and(|profile| profile.contains(',') || profile.contains('\n') || profile.contains('\r')) {
        return Err("configuration cookies_profile must not contain commas or line breaks".into());
    }
    let mut c = Config {
        bind: bind.clone(),
        moonlight: env::var("MEDIABRIDGE_MOONLIGHT")
            .ok().filter(|path| !path.trim().is_empty())
            .or_else(bundled_moonlight_path)
            .unwrap_or_else(|| if cfg!(windows) { "moonlight.exe" } else { "moonlight" }.into()),
        moonlight_args: Vec::new(),
        ytdlp_cookies_from_browser: env::var("MEDIABRIDGE_COOKIES_FROM_BROWSER").ok().filter(|v| !v.trim().is_empty()),
        cookies_browser: if user.cookies_browser == "auto" && (cfg!(target_os = "linux") || cfg!(windows)) {
            "librewolf".into()
        } else { user.cookies_browser },
        cookies_profile: user.cookies_profile,
        remote_origin_servers,
        default_screen: user.default_screen,
        default_quality: user.default_quality,
        subtitles: user.subtitles,
        preferred_language: user.preferred_language,
    };
    let mut bind_overridden = false;
    let mut a = env::args().skip(1);
    while let Some(flag) = a.next() {
        match flag.as_str() {
            "--bind" => { bind = a.next().ok_or_else(|| format!("missing --bind value\n{}", usage()))?; bind_overridden = true; },
            "--moonlight" => c.moonlight = a.next().ok_or_else(|| format!("missing --moonlight value\n{}", usage()))?,
            "--moonlight-arg" => c.moonlight_args.push(a.next().ok_or_else(|| format!("missing --moonlight-arg value\n{}", usage()))?),
            "--cookies-from-browser" => c.ytdlp_cookies_from_browser = Some(a.next().ok_or_else(|| format!("missing --cookies-from-browser value\n{}", usage()))?),
            "--help" | "-h" => { println!("{}", usage()); std::process::exit(0); }
            _ => return Err(format!("unknown option: {flag}\n{}", usage())),
        }
    }
    if bind_overridden { c.bind = bind; }
    Ok(c)
}

fn detect_default_cookie_browser() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    { return Ok("librewolf".into()); }
    #[cfg(not(target_os = "linux"))]
    {
        #[cfg(windows)]
        let raw = Command::new("reg").args(["query", r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "/v", "ProgId"])
            .output().map_err(|e| format!("could not inspect the Windows default browser: {e}"))?;
        #[cfg(all(unix, not(target_os = "linux")))]
        let raw = Command::new("xdg-settings").args(["get", "default-web-browser"])
            .output().map_err(|e| format!("could not inspect the system default browser (xdg-settings): {e}"))?;
        if !raw.status.success() { return Err("could not determine the system default browser".into()); }
        let detected = String::from_utf8_lossy(&raw.stdout).to_ascii_lowercase();
        let browser = if detected.contains("firefox") { "firefox" }
            else if detected.contains("edge") || detected.contains("msedge") { "edge" }
            else if detected.contains("brave") { "brave" }
            else if detected.contains("vivaldi") { "vivaldi" }
            else if detected.contains("opera") { "opera" }
            else if detected.contains("chromium") { "chromium" }
            else if detected.contains("chrome") { "chrome" }
            else if detected.contains("whale") { "whale" }
            else { return Err(format!("the system default browser is not supported by yt-dlp: {}", detected.trim())); };
        Ok(browser.to_string())
    }
}

fn cookie_browser_spec(browser: &str, profile: Option<&str>) -> Result<Option<String>, String> {
    if browser == "off" { return Ok(None); }
    let resolved = if browser == "auto" { detect_default_cookie_browser()? } else { browser.to_string() };
    let profile = profile.map(str::trim).filter(|profile| !profile.is_empty());
    #[cfg(target_os = "linux")]
    if resolved == "librewolf" {
        let profile = env::var_os("MEDIABRIDGE_LIBREWOLF_PROFILE").map(PathBuf::from)
            .or_else(|| env::var_os("XDG_DATA_HOME").map(|home| PathBuf::from(home).join("mediabridge/librewolf-profile")))
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/mediabridge/librewolf-profile")))
            .ok_or("could not locate the MediaBridge LibreWolf profile")?;
        if !profile.join("cookies.sqlite").is_file() {
            eprintln!("LibreWolf has no cookies database yet; trying local playback without browser cookies. Sign in through the MediaBridge LibreWolf profile to enable authenticated playback.");
            return Ok(None);
        }
        return Ok(Some(format!("firefox:{}", profile.display())));
    }
    #[cfg(windows)]
    if resolved == "librewolf" {
        let profile = managed_librewolf_profile()
            .ok_or("could not locate the MediaBridge LibreWolf profile")?;
        return Ok(Some(format!("firefox:{}", profile.display())));
    }
    Ok(Some(match profile { Some(profile) => format!("{resolved}:{profile}"), None => resolved }))
}

#[cfg(windows)]
fn managed_librewolf_profile() -> Option<PathBuf> {
    env::current_exe().ok()?.parent().map(|dir| dir.join("librewolf-profile"))
}

#[cfg(windows)]
fn ensure_librewolf_profile(profile: &Path) -> Result<(), String> {
    std::fs::create_dir_all(profile).map_err(|error| format!("could not create the persistent LibreWolf profile: {error}"))?;
    let prefs = profile.join("user.js");
    let existing = std::fs::read_to_string(&prefs).unwrap_or_default();
    let required = [
        ("privacy.clearOnShutdown.cookies", "user_pref(\"privacy.clearOnShutdown.cookies\", false);\n"),
        ("privacy.clearOnShutdown_v2.cookiesAndStorage", "user_pref(\"privacy.clearOnShutdown_v2.cookiesAndStorage\", false);\n"),
        ("privacy.sanitize.sanitizeOnShutdown", "user_pref(\"privacy.sanitize.sanitizeOnShutdown\", false);\n"),
        ("media.autoplay.default", "user_pref(\"media.autoplay.default\", 0);\n"),
        ("media.autoplay.blocking_policy", "user_pref(\"media.autoplay.blocking_policy\", 0);\n"),
        ("browser.aboutwelcome.enabled", "user_pref(\"browser.aboutwelcome.enabled\", false);\n"),
        ("browser.shell.checkDefaultBrowser", "user_pref(\"browser.shell.checkDefaultBrowser\", false);\n"),
        ("browser.startup.firstrunSkipsHomepage", "user_pref(\"browser.startup.firstrunSkipsHomepage\", true);\n"),
        ("security.insecure_connection_text.enabled", "user_pref(\"security.insecure_connection_text.enabled\", false);\n"),
        // Keep LibreWolf's generic Firefox user-agent for sign-in compatibility.
        // This applies only to MediaBridge's managed profile.
        ("privacy.resistFingerprinting\", true", "user_pref(\"privacy.resistFingerprinting\", true);\n"),
        ("devtools.debugger.remote-enabled", "user_pref(\"devtools.debugger.remote-enabled\", true);\n"),
    ];
    let missing = required.iter().filter(|(key, _)| !existing.contains(key)).map(|(_, line)| *line).collect::<String>();
    if !missing.is_empty() {
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&prefs)
            .map_err(|error| format!("could not configure the persistent LibreWolf profile: {error}"))?;
        if existing.is_empty() { file.write_all(b"// MediaBridge browser settings.\n").ok(); }
        file.write_all(missing.as_bytes()).map_err(|error| format!("could not configure the persistent LibreWolf profile: {error}"))?;
    }
    Ok(())
}

fn configured_cookie_source(cfg: &Config) -> (String, Option<String>) {
    if let Some(value) = cfg.ytdlp_cookies_from_browser.as_deref() {
        let (browser, profile) = value.split_once(':').map_or((value, None), |(browser, profile)| (browser, Some(profile.to_string())));
        return (browser.to_ascii_lowercase(), profile);
    }
    (cfg.cookies_browser.clone(), cfg.cookies_profile.clone())
}

struct CookieCheckTempDir(PathBuf);

impl Drop for CookieCheckTempDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn cookie_check_temp_dir() -> Result<CookieCheckTempDir, String> {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let path = env::temp_dir().join(format!("mediabridge-cookie-check-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).map_err(|error| format!("could not create a private temporary folder for the YouTube profile check: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not restrict access to the temporary cookie folder: {error}"))?;
    }
    Ok(CookieCheckTempDir(path))
}

fn extract_youtube_web_config(html: &str) -> Option<serde_json::Value> {
    let mut offset = 0;
    while let Some(found) = html[offset..].find("ytcfg.set(") {
        let start = offset + found + "ytcfg.set(".len();
        if let Some(open) = html[start..].find('{') {
            let object_start = start + open;
            let bytes = html.as_bytes();
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            for index in object_start..bytes.len() {
                let byte = bytes[index];
                if in_string {
                    if escaped { escaped = false; }
                    else if byte == b'\\' { escaped = true; }
                    else if byte == b'"' { in_string = false; }
                    continue;
                }
                match byte {
                    b'"' => in_string = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            if let Ok(config) = serde_json::from_slice::<serde_json::Value>(&bytes[object_start..=index]) {
                                if config.get("INNERTUBE_API_KEY").and_then(|value| value.as_str()).is_some()
                                    && config.get("INNERTUBE_CONTEXT").is_some() { return Some(config); }
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        offset = start;
        if offset >= html.len() { break; }
    }
    None
}

fn active_youtube_account(value: &serde_json::Value) -> Option<(String, Option<String>)> {
    let mut stack = vec![value];
    while let Some(node) = stack.pop() {
        if let Some(header) = node.get("activeAccountHeaderRenderer") {
            let account_name = header.get("accountName");
            let name = account_name.and_then(|name| name.as_str()
                .or_else(|| name.get("simpleText").and_then(|value| value.as_str()))
                .or_else(|| name.get("runs").and_then(|runs| runs.as_array()).and_then(|runs| runs.iter().find_map(|run| run.get("text").and_then(|text| text.as_str())))));
            let photo = header.get("accountPhoto").or_else(|| header.get("accountAvatar"))
                .and_then(|photo| photo.get("thumbnails"))
                .and_then(|thumbnails| thumbnails.as_array()).and_then(|thumbnails| thumbnails.last())
                .and_then(|thumbnail| thumbnail.get("url")).and_then(|url| url.as_str())
                .map(|url| if url.starts_with("//") { format!("https:{url}") } else { url.to_string() });
            if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
                return Some((name.to_string(), photo));
            }
        }
        match node {
            serde_json::Value::Array(items) => stack.extend(items.iter()),
            serde_json::Value::Object(fields) => stack.extend(fields.values()),
            _ => {}
        }
    }
    None
}

fn youtube_initial_data(html: &str) -> Option<serde_json::Value> {
    for marker in ["ytInitialData =", "window[\"ytInitialData\"] ="] {
        let Some(marker_start) = html.find(marker) else { continue; };
        let start = marker_start + marker.len();
        let bytes = html.as_bytes();
        let Some(object_start) = (start..bytes.len()).find(|index| bytes[*index] == b'{') else { continue; };
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for index in object_start..bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escaped { escaped = false; }
                else if byte == b'\\' { escaped = true; }
                else if byte == b'"' { in_string = false; }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(value) = serde_json::from_slice(&bytes[object_start..=index]) { return Some(value); }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn youtube_account_from_cookies(cookie_file: &Path) -> Result<Option<(String, Option<String>)>, String> {
    let contents = std::fs::read_to_string(cookie_file).map_err(|error| format!("could not read the temporary browser cookie export: {error}"))?;
    let mut google_lines = Vec::new();
    let mut sapisid = None;
    for line in contents.lines() {
        if line.starts_with('#') && !line.starts_with("#HttpOnly_") { continue; }
        let fields = line.trim_start_matches("#HttpOnly_").split('\t').collect::<Vec<_>>();
        if fields.len() < 7 { continue; }
        let domain = fields[0].trim_start_matches('.').to_ascii_lowercase();
        // YouTube login sessions often depend on cookies scoped to google.com
        // as well as youtube.com. Keep both domains for the account lookup.
        if domain == "youtube.com" || domain.ends_with(".youtube.com")
            || domain == "google.com" || domain.ends_with(".google.com") {
            google_lines.push(line.to_string());
        }
        if domain == "youtube.com" || domain.ends_with(".youtube.com")
            || domain == "google.com" || domain.ends_with(".google.com") {
            if matches!(fields[5], "SAPISID" | "__Secure-3PAPISID" | "__Secure-1PAPISID") {
                sapisid = Some(fields[6].to_string());
            }
        }
    }
    if google_lines.is_empty() { return Ok(None); }
    // Only send Google and YouTube cookies to YouTube. Keep this temporary
    // export intact: the account lookup needs both domains and the file is
    // removed by CookieCheckTempDir when this request completes.
    let account_cookie_file = cookie_file.with_file_name("youtube-account-cookies.txt");
    std::fs::write(&account_cookie_file, format!("# Netscape HTTP Cookie File\n{}\n", google_lines.join("\n")))
        .map_err(|error| format!("could not prepare the temporary YouTube account cookie export: {error}"))?;
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let page = Command::new(curl).args(["--fail", "--silent", "--show-error", "--location", "--max-time", "20", "--cookie"])
        .arg(&account_cookie_file).arg("https://www.youtube.com/").output()
        .map_err(|error| format!("could not request YouTube's account page: {error}"))?;
    if !page.status.success() { return Err(format!("YouTube account page request failed ({})", page.status)); }
    let html = String::from_utf8_lossy(&page.stdout);
    if let Some(account) = youtube_initial_data(&html).as_ref().and_then(active_youtube_account) {
        return Ok(Some(account));
    }
    let Some(config) = extract_youtube_web_config(&html) else { return Ok(None); };
    let api_key = config.get("INNERTUBE_API_KEY").and_then(|value| value.as_str()).ok_or("YouTube's page did not provide its account-menu request key")?;
    let context = config.get("INNERTUBE_CONTEXT").cloned().ok_or("YouTube's page did not provide its account-menu context")?;
    let client_version = context.get("client").and_then(|client| client.get("clientVersion")).and_then(|value| value.as_str()).unwrap_or("2.20260101.00.00");
    let request_body = serde_json::json!({"context": context});
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut command = Command::new(curl);
    command.args(["--fail", "--silent", "--show-error", "--location", "--max-time", "20", "--cookie"])
        .arg(&account_cookie_file)
        .args(["--header", "Content-Type: application/json", "--header", "X-YouTube-Client-Name: 1",
            "--header", "Origin: https://www.youtube.com", "--header", "Referer: https://www.youtube.com/"])
        .arg("--header").arg(format!("X-YouTube-Client-Version: {client_version}"));
    if let Some(sapisid) = sapisid {
        let digest = Sha1::digest(format!("{timestamp} {sapisid} https://www.youtube.com").as_bytes());
        let hex = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        command.arg("--header").arg(format!("Authorization: SAPISIDHASH {timestamp}_{hex}"));
    }
    let endpoint = format!("https://www.youtube.com/youtubei/v1/account/account_menu?key={api_key}");
    let body = request_body.to_string();
    let response = command.arg("--data-binary").arg(body).arg(endpoint).output()
        .map_err(|error| format!("could not request YouTube's account menu: {error}"))?;
    if !response.status.success() { return Ok(None); }
    let account: serde_json::Value = match serde_json::from_slice(&response.stdout) { Ok(value) => value, Err(_) => return Ok(None) };
    Ok(active_youtube_account(&account))
}

fn check_cookie_access(browser: &str, profile: Option<&str>, site: &str) -> Result<serde_json::Value, String> {
    let spec = cookie_browser_spec(browser, profile)?.ok_or("browser cookie access is turned off")?;
    let temp_dir = cookie_check_temp_dir()?;
    let cookie_file = temp_dir.0.join(format!("{site}-cookies.txt"));
    let mut command = media_tool_command("yt-dlp");
    command.arg("--cookies-from-browser").arg(&spec).arg("--cookies").arg(&cookie_file);
    if site == "twitch" {
        // The generated Netscape cookie export is inspected for Twitch's auth
        // cookie. This confirms that yt-dlp can read the profile and that a
        // Twitch session cookie exists, but cannot prove the session is valid.
        command.args(["--skip-download", "--simulate", "--no-warnings", "https://www.twitch.tv/twitch"]);
    } else {
        command.args(["--dump-single-json", "--skip-download", "--no-playlist", "--no-warnings", "--remote-components", "ejs:github", "https://www.youtube.com/watch?v=dQw4w9WgXcQ"]);
    }
    add_deno_to_path(&mut command);
    let output = command.output().map_err(|e| format!("could not run yt-dlp to check browser cookies: {e}"))?;
    if site == "twitch" {
        let extractor_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let lower_error = extractor_error.to_ascii_lowercase();
        if lower_error.contains("failed to decrypt with dpapi") {
            return Err("Windows could not decrypt this browser profile's cookies through DPAPI. Try the managed LibreWolf profile and make sure MediaBridge runs as the same Windows user that owns it.".into());
        }
        if lower_error.contains("could not copy chrome cookie database") {
            return Err("Could not read the Chromium browser cookie database. Sign in with the managed LibreWolf profile, or fully quit Chrome/Edge and retry.".into());
        }
        let contents = std::fs::read_to_string(&cookie_file).map_err(|_| {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let lower_error = error.to_ascii_lowercase();
            if lower_error.contains("failed to decrypt with dpapi") {
                "Windows could not decrypt this browser profile's cookies through DPAPI. Try the managed LibreWolf profile and make sure MediaBridge runs as the same Windows user that owns it.".to_string()
            } else if lower_error.contains("could not copy chrome cookie database") {
                "Could not read the Chromium browser cookie database. Sign in with the managed LibreWolf profile, or fully quit Chrome/Edge and retry.".to_string()
            } else if error.is_empty() {
                "yt-dlp could not export cookies from this browser profile.".to_string()
            } else { error }
        })?;
        let auth = contents.lines().filter(|line| !line.starts_with('#') || line.starts_with("#HttpOnly_"))
            .filter_map(|line| { let fields = line.trim_start_matches("#HttpOnly_").split('\t').collect::<Vec<_>>();
                let domain = fields.first().map(|value| value.trim_start_matches('.').to_ascii_lowercase()).unwrap_or_default();
                (fields.len() >= 7 && (domain == "twitch.tv" || domain.ends_with(".twitch.tv")))
                    .then(|| (fields[5], fields[6])) }).find(|(name, _)| *name == "auth-token");
        let login = contents.lines().filter(|line| !line.starts_with('#') || line.starts_with("#HttpOnly_"))
            .filter_map(|line| { let fields = line.trim_start_matches("#HttpOnly_").split('\t').collect::<Vec<_>>();
                let domain = fields.first().map(|value| value.trim_start_matches('.').to_ascii_lowercase()).unwrap_or_default();
                (fields.len() >= 7 && (domain == "twitch.tv" || domain.ends_with(".twitch.tv")) && fields[5] == "login")
                    .then(|| fields[6].to_string()) }).next();
        if auth.is_none() {
            return Err("yt-dlp could read this browser profile, but no Twitch auth cookie was found. Sign in to Twitch in this browser and retry.".into());
        }
        #[cfg(target_os = "linux")]
        let browser_label = if browser == "librewolf" || browser == "auto" { "MediaBridge LibreWolf".to_string() } else { spec.clone() };
        #[cfg(not(target_os = "linux"))]
        let browser_label = spec.clone();
        return Ok(serde_json::json!({"ok": true, "site": "twitch", "browser": browser_label,
            "cookie_found": true, "profile_name": login, "profile_avatar_url": null}));
    }
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let lower_error = error.to_ascii_lowercase();
        if lower_error.contains("failed to decrypt with dpapi") {
            return Err("Windows could not decrypt this browser profile's cookies through DPAPI. Closing the browser will not fix this. Try a Firefox profile signed into YouTube, and make sure MediaBridge runs as the same Windows user that owns the browser profile. Chromium-based browser encryption may not be readable by yt-dlp on this setup.".into());
        }
        if lower_error.contains("could not copy chrome cookie database") {
            return Err("Windows could not copy the Chromium browser's cookie database because it is locked or inaccessible. Fully quit Chrome/Edge/Brave/Chromium (including background processes), then retry. MediaBridge will not close your browser automatically; Firefox may be an alternative profile to check.".into());
        }
        return Err(if error.is_empty() { format!("yt-dlp cookie check failed ({})", output.status) } else { error });
    }
    if site == "twitch" {
        return Err("yt-dlp could not read Twitch cookies from this browser profile.".into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("yt-dlp read cookies but could not confirm a YouTube video: {e}"))?;
    let title = metadata.get("title").and_then(|value| value.as_str()).unwrap_or("YouTube video");
    let (account, account_lookup_error) = match youtube_account_from_cookies(&cookie_file) {
        Ok(account) => (account, None),
        Err(error) => (None, Some(error)),
    };
    #[cfg(target_os = "linux")]
    let browser_label = if browser == "librewolf" || browser == "auto" { "MediaBridge LibreWolf".to_string() } else { spec.clone() };
    #[cfg(not(target_os = "linux"))]
    let browser_label = spec.clone();
    Ok(serde_json::json!({"ok": true, "site": "youtube", "browser": browser_label, "title": title,
        "channel_name": account.as_ref().map(|account| account.0.as_str()),
        "channel_avatar_url": account.as_ref().and_then(|account| account.1.as_deref()),
        "channel_lookup_error": account_lookup_error}))
}

fn expand(args: &[String], value: &str, placeholder: &str) -> Vec<String> {
    args.iter().map(|arg| arg.replace(placeholder, value)).collect()
}

// yt-dlp's child process needs to see Deno even when MediaBridge was started
// from a desktop launcher whose PATH omits the per-user Deno install directory.
fn add_deno_to_path(command: &mut Command) {
    let mut candidates = Vec::new();
    // The Windows installer keeps Deno beside MediaBridge so the package is
    // self-contained and can be removed without touching the user's PATH.
    #[cfg(windows)]
    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.to_path_buf());
        }
    }
    if let Some(install_dir) = env::var_os("DENO_INSTALL") {
        candidates.push(PathBuf::from(install_dir).join("bin"));
    }
    if let Some(home) = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        candidates.push(PathBuf::from(home).join(".deno").join("bin"));
    }
    let deno_executable = if cfg!(windows) { "deno.exe" } else { "deno" };
    let Some(deno_dir) = candidates.into_iter().find(|dir| dir.join(deno_executable).is_file()) else {
        return;
    };
    let current_path = env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![deno_dir];
    paths.extend(env::split_paths(&current_path));
    if let Ok(path) = env::join_paths(paths) {
        command.env("PATH", path);
    }
}

fn media_tool_command(name: &str) -> Command {
    #[cfg(windows)]
    {
        let sibling = env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.join(format!("{name}.exe"))));
        if let Some(path) = sibling.filter(|path| path.is_file()) {
            let mut command = Command::new(path);
            windows_hide_console(&mut command);
            return command;
        }
    }
    let command = Command::new(name);
    #[cfg(windows)]
    {
        let mut command = command;
        windows_hide_console(&mut command);
        return command;
    }
    #[cfg(not(windows))]
    command
}

fn browser_candidates(browser: &str) -> Vec<String> {
    let executable = Path::new(browser).file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| browser.to_lowercase());
    if browser.contains('/') || browser.contains('\\') {
        return vec![browser.to_string()];
    }
    #[cfg(target_os = "linux")]
    if executable == "librewolf" {
        let launcher = env::var_os("MEDIABRIDGE_LIBREWOLF_LAUNCHER")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/bin/mediabridge-librewolf")));
        return launcher.into_iter().map(|path| path.to_string_lossy().into_owned()).collect();
    }
    #[cfg(windows)]
    if executable == "librewolf" {
        let browser = env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("librewolf/librewolf.exe")));
        return browser.into_iter().map(|path| path.to_string_lossy().into_owned()).collect();
    }
    let candidates = match executable.as_str() {
        "chromium" | "chromium-browser" => vec![browser.to_string(), "chromium".into(), "chromium-browser".into()],
        "google-chrome" | "google-chrome-stable" => vec![browser.to_string(), "google-chrome".into(), "google-chrome-stable".into()],
        _ => vec![browser.to_string()],
    };
    #[cfg(windows)]
    let candidates = {
        let mut candidates = candidates;
        let locations = match executable.as_str() {
            "chromium" | "chromium-browser" | "google-chrome" | "google-chrome-stable" => vec![
                env::var_os("PROGRAMFILES").map(PathBuf::from).map(|p| p.join("Google/Chrome/Application/chrome.exe")),
                env::var_os("PROGRAMFILES(X86)").map(PathBuf::from).map(|p| p.join("Google/Chrome/Application/chrome.exe")),
                env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("Google/Chrome/Application/chrome.exe")),
            ],
            "firefox" => vec![
                env::var_os("PROGRAMFILES").map(PathBuf::from).map(|p| p.join("Mozilla Firefox/firefox.exe")),
                env::var_os("PROGRAMFILES(X86)").map(PathBuf::from).map(|p| p.join("Mozilla Firefox/firefox.exe")),
            ],
            _ => Vec::new(),
        };
        let installed = locations.into_iter().flatten().filter(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned()).collect::<Vec<_>>();
        candidates.splice(0..0, installed);
        candidates
    };
    candidates
}

fn chromium_name(browser: &str) -> bool {
    let name = Path::new(browser).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["chromium", "chromium-browser", "chromium.exe", "google-chrome", "google-chrome-stable", "chrome.exe"].contains(&name.as_str())
}

fn youtube_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false; };
    if scheme != "https" && scheme != "http" { return false; }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").split(':').next().unwrap_or("").to_lowercase();
    host == "youtube.com" || host.ends_with(".youtube.com")
        || host == "youtube-nocookie.com" || host.ends_with(".youtube-nocookie.com")
        || host == "youtu.be"
}

fn twitch_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false; };
    if scheme != "https" && scheme != "http" { return false; }
    let host = rest.split(['/','?','#']).next().unwrap_or("").split(':').next().unwrap_or("").to_lowercase();
    host == "twitch.tv" || host.ends_with(".twitch.tv")
}

fn website_host(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map(|(_, host)| host).unwrap_or(authority).to_ascii_lowercase();
    Some(authority.strip_prefix("www.").unwrap_or(&authority).to_string())
}

fn website_path(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let path = rest.find(['/', '?', '#']).map(|index| &rest[index..]).unwrap_or("/");
    let path = if path.starts_with('?') { format!("/{path}") } else if path.is_empty() { "/".to_string() } else { path.to_string() };
    Some(path.trim_end_matches('/').to_string())
}

fn same_website_host(left: &str, right: &str) -> bool {
    website_host(left).is_some() && website_host(left) == website_host(right)
}

fn same_website_page(left: &str, right: &str) -> bool {
    same_website_host(left, right) && website_path(left) == website_path(right)
}

fn clean_media_url(input: &str, allow_smb: bool) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.to_ascii_lowercase().starts_with("smb://") {
        if !allow_smb { return Err("SMB links are only supported with local mpv playback".into()); }
        let rest = &trimmed[6..];
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let path = rest.strip_prefix(authority).unwrap_or("");
        if authority.is_empty() || authority.contains('@') || authority.contains('%') || !authority.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']')) {
            return Err("SMB links must use smb://server/share/path and must not contain usernames or passwords".into());
        }
        if authority.contains('?') || authority.contains('#') || path.contains('?') || path.contains('#') {
            return Err("SMB links must not contain query strings or fragments".into());
        }
        let mut parts = path.trim_start_matches('/').split('/');
        if parts.next().filter(|share| !share.is_empty()).is_none() || parts.next().filter(|part| !part.is_empty()).is_none() {
            return Err("SMB links must include a share and a media file path".into());
        }
        return Ok(trimmed.to_string());
    }
    let start = ["https://", "http://"].iter().filter_map(|prefix| trimmed.find(prefix)).min();
    let candidate = if let Some(start) = start { &trimmed[start..] }
        else if ["youtube.com/", "www.youtube.com/", "m.youtube.com/", "youtu.be/", "twitch.tv/", "www.twitch.tv/"].iter().any(|prefix| trimmed.starts_with(prefix)) { trimmed }
        else { return Err("paste a complete http(s) link".into()); };
    let candidate = candidate.split_whitespace().next().unwrap_or("")
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | ';' | '!'));
    let normalized = if candidate.starts_with("http://") || candidate.starts_with("https://") {
        candidate.to_string()
    } else { format!("https://{candidate}") };
    let Some((scheme, rest)) = normalized.split_once("://") else { return Err("link must use http or https".into()); };
    if !matches!(scheme, "http" | "https") { return Err("link must use http or https".into()); }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') { return Err("link has no valid host".into()); }
    Ok(normalized)
}

fn safe_media_url(input: &str) -> String {
    let Some((scheme, rest)) = input.split_once("://") else { return input.to_string(); };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = &rest[authority_end..];
    let authority = if let Some((_, host)) = authority.rsplit_once('@') { format!("<redacted>@{host}") } else { authority.to_string() };
    let mut output = format!("{scheme}://{authority}");
    if let Some((before, query_and_tail)) = suffix.split_once('?') {
        output.push_str(before);
        let (query, tail) = query_and_tail.split_once('#').unwrap_or((query_and_tail, ""));
        let filtered = query.split('&').filter(|part| {
            let key = part.split('=').next().unwrap_or("").to_ascii_lowercase();
            !["token", "access_token", "api_key", "apikey", "password", "passwd", "secret", "signature", "sig", "auth", "credential", "session"].iter().any(|s| key == *s || key.contains(s))
        }).collect::<Vec<_>>().join("&");
        if !filtered.is_empty() { output.push('?'); output.push_str(&filtered); }
        if !tail.is_empty() { output.push('#'); output.push_str(tail); }
    } else { output.push_str(suffix); }
    output
}

fn run_cancellable_command(mut command: Command, cancel: &AtomicBool) -> Result<std::process::Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    isolate_process_group(&mut command);
    let mut child = command.spawn().map_err(|error| format!("could not start background media lookup: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("background lookup stdout was unavailable")?;
    let mut stderr = child.stderr.take().ok_or("background lookup stderr was unavailable")?;
    let stdout_reader = thread::spawn(move || { let mut bytes = Vec::new(); let _ = stdout.read_to_end(&mut bytes); bytes });
    let stderr_reader = thread::spawn(move || { let mut bytes = Vec::new(); let _ = stderr.read_to_end(&mut bytes); bytes });
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            terminate_child(&mut child);
            let _ = stdout_reader.join(); let _ = stderr_reader.join();
            return Err("background media lookup was cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => { terminate_child(&mut child); let _ = stdout_reader.join(); let _ = stderr_reader.join(); return Err(format!("could not check background lookup process: {error}")); }
        }
    };
    Ok(std::process::Output { status, stdout: stdout_reader.join().unwrap_or_default(), stderr: stderr_reader.join().unwrap_or_default() })
}

fn resolve_media_url(url: &str) -> Result<serde_json::Value, String> { resolve_media_url_with_cancel(url, None) }

fn resolve_media_url_with_cancel(url: &str, cancel: Option<&AtomicBool>) -> Result<serde_json::Value, String> {
    let mut command = media_tool_command("yt-dlp");
    command.args(["--dump-single-json", "--no-warnings", "--skip-download", "--no-playlist", "--remote-components", "ejs:github", url]);
    add_deno_to_path(&mut command);
    let output = if let Some(cancel) = cancel { run_cancellable_command(command, cancel)? }
        else { command.output().map_err(|e| format!("could not run yt-dlp to resolve the link: {e}"))? };
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() { format!("yt-dlp could not resolve the link ({})", output.status) } else { error });
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("yt-dlp returned invalid media metadata: {e}"))?;
    let thumbnails = metadata.get("thumbnails").and_then(|v| v.as_array());
    let video_thumbnail = metadata.get("thumbnail").and_then(|v| v.as_str())
        .or_else(|| thumbnails.and_then(|items| items.iter().rev().find_map(|item| item.get("url").and_then(|v| v.as_str()))));
    let field = |names: &[&str]| names.iter().find_map(|name| metadata.get(*name).and_then(|v| v.as_str()));
    let channel_avatar = field(&["channel_thumbnail", "channel_thumbnail_url", "uploader_thumbnail", "uploader_avatar"]);
    let channel_banner = field(&["channel_banner", "channel_banner_url", "uploader_banner"]);
    Ok(serde_json::json!({
        "url": metadata.get("webpage_url").and_then(|v| v.as_str()).unwrap_or(url),
        "title": metadata.get("title").and_then(|v| v.as_str()),
        "uploader": metadata.get("uploader").or_else(|| metadata.get("channel")).and_then(|v| v.as_str()),
        "extractor": metadata.get("extractor_key").or_else(|| metadata.get("extractor")).and_then(|v| v.as_str()),
        "thumbnail_url": video_thumbnail,
        "channel_avatar_url": channel_avatar,
        "channel_banner_url": channel_banner,
        "duration_seconds": metadata.get("duration").and_then(|v| v.as_f64()),
        "is_live": metadata.get("is_live").and_then(|v| v.as_bool()).unwrap_or(false)
    }))
}

fn expand_youtube_playlist(url: &str) -> Result<Vec<String>, String> {
    let mut command = media_tool_command("yt-dlp");
    command.args(["--flat-playlist", "--dump-single-json", "--no-warnings", "--skip-download", url]);
    add_deno_to_path(&mut command);
    let output = command.output().map_err(|e| format!("could not run yt-dlp to read the playlist: {e}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() { format!("yt-dlp could not read the playlist ({})", output.status) } else { error });
    }
    parse_youtube_playlist_json(&output.stdout)
}

fn parse_youtube_playlist_json(output: &[u8]) -> Result<Vec<String>, String> {
    let metadata: serde_json::Value = serde_json::from_slice(output)
        .map_err(|e| format!("yt-dlp returned invalid playlist metadata: {e}"))?;
    let entries = metadata.get("entries").and_then(|v| v.as_array()).ok_or("the link did not resolve to a video playlist")?;
    let urls = entries.iter().filter_map(|entry| {
        if entry.is_null() { return None; }
        let id = entry.get("id").and_then(|v| v.as_str())
            .or_else(|| entry.get("url").and_then(|v| v.as_str()).filter(|value| value.len() == 11));
        id.filter(|id| id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
            .map(|id| format!("https://www.youtube.com/watch?v={id}"))
    }).collect::<Vec<_>>();
    if urls.is_empty() { return Err("the playlist contains no available videos".into()); }
    Ok(urls)
}

fn expand_youtube_mix_cancellable(video_url: &str, cancel: &AtomicBool) -> Result<Vec<String>, String> {
    let id = youtube_video_id(video_url).ok_or("could not extract a YouTube video ID for its Mix")?;
    let url = format!("https://www.youtube.com/watch?v={id}&list=RD{id}&start_radio=1");
    let mut command = media_tool_command("yt-dlp");
    command.args(["--flat-playlist", "--dump-single-json", "--no-warnings", "--skip-download", &url]);
    add_deno_to_path(&mut command);
    let output = run_cancellable_command(command, cancel)?;
    if !output.status.success() {
        let status = output.status;
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() { format!("yt-dlp could not read the YouTube Mix ({status})") } else { error });
    }
    parse_youtube_playlist_json(&output.stdout)
}

fn update_item_metadata(item: &mut QueueItem, metadata: &serde_json::Value) {
    let text = |key: &str| metadata.get(key).and_then(|value| value.as_str()).map(str::to_string);
    if let Some(value) = text("title") { item.title = Some(value); }
    if let Some(value) = text("uploader") { item.uploader = Some(value); }
    if let Some(value) = text("thumbnail_url") { item.thumbnail_url = Some(value); }
    if let Some(value) = text("channel_avatar_url") { item.channel_avatar_url = Some(value); }
    if let Some(value) = text("channel_banner_url") { item.channel_banner_url = Some(value); }
}

fn same_media_url(a: &str, b: &str) -> bool {
    a == b || matches!((youtube_video_id(a), youtube_video_id(b)), (Some(a), Some(b)) if a == b)
}

fn drain_metadata(state: &mut State) {
    let results: Vec<(String, Result<serde_json::Value, String>)> = state.metadata_rx.as_ref()
        .map(|receiver| receiver.try_iter().collect()).unwrap_or_default();
    for (url, result) in results {
        if state.metadata_in_flight_url.as_deref() == Some(&url) {
            state.metadata_in_flight = false;
            state.metadata_cancel = None;
            state.metadata_in_flight_url = None;
        }
        match result {
            Ok(metadata) => {
                if let Some(item) = state.current.as_mut().filter(|item| same_media_url(&item.url, &url)) {
                    update_item_metadata(item, &metadata);
                }
                for item in state.queue.iter_mut().chain(state.history.iter_mut()) {
                    if same_media_url(&item.url, &url) { update_item_metadata(item, &metadata); }
                }
            }
            Err(error) => eprintln!("could not resolve background metadata for {}: {error}", safe_media_url(&url)),
        }
    }
}

fn dispatch_pending_metadata(state: &mut State) {
    if state.metadata_in_flight { return; }
    // Let mpv finish opening its stream before running another yt-dlp process;
    // otherwise metadata extraction competes with the playback extractor.
    let url = if let Some(url) = state.pending_metadata.pop_front() {
        if state.current.as_ref().is_some_and(|item| item.local_playback) {
            let Some(socket) = state.mpv_socket.as_deref() else { state.pending_metadata.push_front(url); return; };
            if mpv_property(socket, "time-pos").is_none()
                || mpv_property(socket, "pause").and_then(|value| value.as_bool()) != Some(false) {
                state.pending_metadata.push_front(url);
                return;
            }
        }
        url
    } else {
        if state.pending_queue_metadata.is_empty() || !current_playback_started(state) { return; }
        let Some(url) = state.pending_queue_metadata.pop_front() else { return; };
        url
    };
    let Some(metadata_tx) = state.metadata_tx.clone() else { return; };
    let cancel = Arc::new(AtomicBool::new(false));
    state.metadata_cancel = Some(cancel.clone());
    state.metadata_in_flight_url = Some(url.clone());
    state.metadata_in_flight = true;
    thread::spawn(move || {
        let result = resolve_media_url_with_cancel(&url, Some(&cancel));
        let _ = metadata_tx.send((url, result));
    });
}

fn drain_autoplay_results(state: &mut State) {
    let results: Vec<(String, Result<Vec<String>, String>)> = state.autoplay_rx.as_ref()
        .map(|receiver| receiver.try_iter().collect()).unwrap_or_default();
    for (seed, result) in results {
        if state.autoplay_in_flight.as_deref() == Some(&seed) { state.autoplay_in_flight = None; state.autoplay_cancel = None; }
        if !state.autoplay_enabled || !state.current.as_ref().is_some_and(|item| same_media_url(&item.url, &seed)) { continue; }
        match result {
            Ok(urls) => {
                let Some(template) = state.current.clone() else { continue; };
                let mut added = 0usize;
                for url in urls {
                    let Some(id) = youtube_video_id(&url) else { continue; };
                    if state.autoplay_seen_ids.contains(&id)
                        || state.current.as_ref().is_some_and(|item| same_media_url(&item.url, &url))
                        || state.queue.iter().any(|item| same_media_url(&item.url, &url))
                        || state.history.iter().any(|item| same_media_url(&item.url, &url)) { continue; }
                    state.autoplay_seen_ids.insert(id);
                    let mut item = template.clone();
                    item.url = url;
                    item.title = None; item.uploader = None; item.thumbnail_url = None;
                    item.channel_avatar_url = None; item.channel_banner_url = None;
                    item.autoplay_generated = true;
                    state.pending_queue_metadata.push_back(item.url.clone());
                    enqueue_autoplay_item(state, item);
                    added += 1;
                    if added >= 20 { break; }
                }
                if added > 0 { println!("Added {added} YouTube Mix videos after {}", safe_media_url(&seed)); }
                else { eprintln!("YouTube Mix for {} had no new available videos", safe_media_url(&seed)); }
            }
            Err(error) => eprintln!("could not expand the YouTube Mix for {}: {error}", safe_media_url(&seed)),
        }
    }
}

fn dispatch_autoplay_mix(state: &mut State) {
    if !state.autoplay_enabled || !state.queue.is_empty() || state.autoplay_in_flight.is_some()
        || !current_playback_started(state) { return; }
    let Some(seed) = state.current.as_ref().filter(|item| youtube_url(&item.url)).map(|item| item.url.clone()) else { return; };
    request_autoplay_mix(state, seed);
}

fn request_autoplay_mix(state: &mut State, seed: String) {
    if !state.autoplay_enabled || !state.queue.is_empty() || state.autoplay_in_flight.is_some() { return; }
    let Some(id) = youtube_video_id(&seed) else { return; };
    if state.autoplay_requested_seed.as_deref() == Some(&id) { return; }
    let Some(tx) = state.autoplay_tx.clone() else { return; };
    state.autoplay_requested_seed = Some(id.clone());
    state.autoplay_seen_ids.insert(id);
    state.autoplay_in_flight = Some(seed.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    state.autoplay_cancel = Some(cancel.clone());
    thread::spawn(move || { let result = expand_youtube_mix_cancellable(&seed, &cancel); let _ = tx.send((seed, result)); });
}

fn enqueue_autoplay_item(state: &mut State, item: QueueItem) { state.queue.push_back(item); }

fn enqueue_explicit_item(state: &mut State, item: QueueItem) {
    state.queue.retain(|queued| !queued.autoplay_generated);
    state.queue.push_back(item);
}

fn set_autoplay(state: &mut State, enabled: bool, duration_minutes: Option<u64>) {
    state.autoplay_enabled = enabled;
    state.autoplay_until = if enabled { duration_minutes.map(|minutes| std::time::Instant::now() + Duration::from_secs(minutes * 60)) } else { None };
    if !enabled {
        if let Some(cancel) = state.autoplay_cancel.take() { cancel.store(true, Ordering::Release); }
        state.autoplay_in_flight = None;
        state.autoplay_waiting_to_advance = false;
        state.queue.retain(|item| !item.autoplay_generated);
        state.autoplay_requested_seed = None;
    } else {
        state.autoplay_requested_seed = None;
    }
}

fn current_playback_started(state: &mut State) -> bool {
    let Some((local_playback, browser)) = state.current.as_ref().map(|item| (item.local_playback, item.browser.clone())) else { return false; };
    if local_playback {
        return state.mpv_socket.as_deref().is_some_and(|socket| {
            mpv_property(socket, "time-pos").is_some()
                && mpv_property(socket, "pause").and_then(|value| value.as_bool()) == Some(false)
        });
    }
    #[cfg(any(windows, target_os = "linux"))]
    if browser.as_deref().is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf")) {
        return refresh_librewolf_video(state).is_some_and(|video| video.paused == Some(false));
    }
    #[cfg(target_os = "linux")]
    if let Some(player) = browser_player(browser.as_deref()) {
        return run_playerctl(&player, &["status"]).is_ok_and(|status| status.trim().eq_ignore_ascii_case("playing"));
    }
    false
}

fn isolate_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

#[cfg(windows)]
fn raise_mpv_window(process_id: u32) {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, FlashWindow, GetForegroundWindow,
        GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow,
        SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SW_RESTORE,
    };

    struct SearchWindow { process_id: u32, window: HWND }

    unsafe extern "system" fn find_process_window(hwnd: HWND, parameter: LPARAM) -> BOOL {
        let search = unsafe { &mut *(parameter as *mut SearchWindow) };
        let mut owner_process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner_process_id); }
        if owner_process_id == search.process_id && unsafe { IsWindowVisible(hwnd) } != 0 {
            search.window = hwnd;
            0
        } else {
            1
        }
    }

    // mpv can take a short time to create its native window after spawning.
    for _ in 0..50 {
        let mut search = SearchWindow { process_id, window: std::ptr::null_mut() };
        unsafe { EnumWindows(Some(find_process_window), &mut search as *mut _ as LPARAM); }
        if !search.window.is_null() {
            unsafe {
                let foreground = GetForegroundWindow();
                let foreground_thread = if foreground.is_null() {
                    0
                } else {
                    GetWindowThreadProcessId(foreground, std::ptr::null_mut())
                };
                let current_thread = GetCurrentThreadId();
                let attached = foreground_thread != 0 && foreground_thread != current_thread
                    && AttachThreadInput(current_thread, foreground_thread, 1) != 0;
                ShowWindow(search.window, SW_RESTORE);
                BringWindowToTop(search.window);
                let activated = SetForegroundWindow(search.window) != 0;
                if attached { AttachThreadInput(current_thread, foreground_thread, 0); }
                if !activated {
                    SetWindowPos(
                        search.window, HWND_TOPMOST, 0, 0, 0, 0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                    );
                    FlashWindow(search.window, 1);
                    append_windows_log("Windows blocked mpv foreground activation; raised it above other windows and flashed its taskbar button".into());
                }
            }
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
fn restore_librewolf_window(process_id: u32, display_index: Option<usize>) {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
        SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOZORDER,
        SWP_NOSIZE, SWP_SHOWWINDOW, SW_MAXIMIZE, SW_RESTORE,
    };

    struct WindowSearch { process_id: u32, window: HWND }
    unsafe extern "system" fn find_window(hwnd: HWND, parameter: LPARAM) -> BOOL {
        let search = unsafe { &mut *(parameter as *mut WindowSearch) };
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner); }
        if owner == search.process_id && unsafe { IsWindowVisible(hwnd) } != 0 {
            search.window = hwnd;
            0
        } else { 1 }
    }

    unsafe extern "system" fn collect_monitor(monitor: HMONITOR, _: HDC, _: *mut RECT, parameter: LPARAM) -> BOOL {
        let monitors = unsafe { &mut *(parameter as *mut Vec<RECT>) };
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 { monitors.push(info.rcMonitor); }
        1
    }

    let mut monitors: Vec<RECT> = Vec::new();
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(collect_monitor), &mut monitors as *mut _ as LPARAM); }
    for _ in 0..60 {
        let mut search = WindowSearch { process_id, window: std::ptr::null_mut() };
        unsafe { EnumWindows(Some(find_window), &mut search as *mut _ as LPARAM); }
        if !search.window.is_null() {
            unsafe {
                ShowWindow(search.window, SW_RESTORE);
                if let Some(rect) = display_index.and_then(|index| monitors.get(index)) {
                    SetWindowPos(search.window, HWND_TOP, rect.left, rect.top, 0, 0,
                        SWP_NOZORDER | SWP_NOSIZE | SWP_SHOWWINDOW);
                }
                ShowWindow(search.window, SW_MAXIMIZE);
                BringWindowToTop(search.window);
                SetForegroundWindow(search.window);
            }
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    append_windows_log("Could not find the new LibreWolf window to restore it from a minimized state".into());
}

fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    {
        let group = -(child.id() as i32);
        // MediaBridge puts each player/browser in its own process group so
        // closing it also closes Chromium/Firefox descendants and wrappers.
        unsafe { kill(group, 15); }
        for _ in 0..15 {
            let _ = child.try_wait();
            if unsafe { kill(group, 0) } != 0 { break; }
            thread::sleep(Duration::from_millis(100));
        }
        if unsafe { kill(group, 0) } == 0 { unsafe { kill(group, 9); } }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill").args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn youtube_tv_url(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else { return false; };
    let path = rest.split(['/', '?', '#']).skip(1).next().unwrap_or("");
    path == "tv"
}

fn youtube_video_id(url: &str) -> Option<String> {
    for part in url.split(['?', '#', '&']) {
        if let Some(id) = part.strip_prefix("v=") {
            let id = id.split(['?', '#', '&']).next().unwrap_or("");
            if !id.is_empty() { return Some(id.to_string()); }
        }
    }
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/')?;
    let host = host.split(':').next()?.to_lowercase();
    let segments = path.split(['?', '#', '&']).next()?.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>();
    if host == "youtu.be" { return segments.first().map(|id| id.to_string()); }
    if ["shorts", "live", "embed"].contains(&segments.first().copied().unwrap_or("")) {
        return segments.get(1).map(|id| id.to_string());
    }
    None
}

fn set_youtube_interface(item: &mut QueueItem) -> Result<(), String> {
    if !youtube_url(&item.url) { return Ok(()); }
    match item.youtube_mode.to_ascii_lowercase().as_str() {
        "regular" => {
            if youtube_tv_url(&item.url) {
                let id = youtube_video_id(&item.url).ok_or("could not extract a video ID from the YouTube TV URL")?;
                item.url = format!("https://www.youtube.com/watch?v={id}");
            }
            Ok(())
        }
        "tv" => {
            let id = youtube_video_id(&item.url).ok_or("YouTube TV mode requires a YouTube video URL with a video ID")?;
            item.url = format!("https://www.youtube.com/tv#/watch?v={id}");
            Ok(())
        }
        _ => Err("youtube_mode must be 'regular' or 'tv'".into()),
    }
}

fn chromium_profile() -> Result<PathBuf, String> {
    let base = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "HOME or XDG_CONFIG_HOME is required for the dedicated Chromium profile".to_string())?;
    let profile = base.join("mediabridge").join("chromium");
    std::fs::create_dir_all(&profile).map_err(|e| format!("could not create Chromium profile {}: {e}", profile.display()))?;
    // Chromium's port=0 discovery depends on DevToolsActivePort being written
    // to the profile. Some distro builds/wrappers announce the chosen port in
    // their log but fail to leave that file, so choose and record the port.
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("could not reserve a local Chromium DevTools port: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("could not read reserved DevTools port: {e}"))?.port();
    drop(listener);
    std::fs::write(profile.join("MediaBridgeDevToolsPort"), port.to_string())
        .map_err(|e| format!("could not record Chromium DevTools port: {e}"))?;
    Ok(profile)
}

fn chromium_debug_port(profile: &Path) -> Result<u16, String> {
    let path = profile.join("MediaBridgeDevToolsPort");
    std::fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?
        .trim().parse::<u16>().map_err(|e| format!("invalid Chromium DevTools port in {}: {e}", path.display()))
}

fn local_http_json(port: u16, path: &str) -> Result<serde_json::Value, String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
        .map_err(|e| format!("could not connect to Chromium DevTools: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes())
        .map_err(|e| format!("could not request Chromium DevTools targets: {e}"))?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).map_err(|e| format!("could not read Chromium DevTools response: {e}"))?;
    let split = response.windows(4).position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "invalid Chromium DevTools HTTP response".to_string())?;
    serde_json::from_slice(&response[split + 4..]).map_err(|e| format!("invalid Chromium DevTools JSON: {e}"))
}

#[cfg(windows)]
fn windows_hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
}

#[cfg(any(windows, target_os = "linux"))]
fn librewolf_bidi_session() -> Result<LibreWolfBidi, String> {
    let (socket, _) = connect("ws://127.0.0.1:9222/session")
        .map_err(|error| format!("could not connect to LibreWolf BiDi at 127.0.0.1:9222: {error}"))?;
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        stream.set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|error| format!("could not set a timeout for LibreWolf BiDi: {error}"))?;
    }
    let mut bidi = LibreWolfBidi { socket, next_id: 0, context_id: None, cached_at: None,
        cached_video: None, autoplay_attempted: false, fullscreen_attempted_at: None };
    bidi.call("session.new", serde_json::json!({"capabilities": {}}))?;
    Ok(bidi)
}

#[cfg(any(windows, target_os = "linux"))]
impl LibreWolfBidi {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        let payload = serde_json::json!({"id": id, "method": method, "params": params}).to_string();
        self.socket.send(Message::Text(payload)).map_err(|error| format!("could not send LibreWolf BiDi command: {error}"))?;
        loop {
            let message = self.socket.read().map_err(|error| format!("could not read LibreWolf BiDi response: {error}"))?;
            if let Message::Text(text) = message {
                let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| format!("invalid LibreWolf BiDi response: {error}"))?;
                if value.get("id").and_then(|value| value.as_u64()) != Some(id) { continue; }
                if value.get("type").and_then(|value| value.as_str()) == Some("error") {
                    return Err(format!("LibreWolf BiDi rejected {method}: {}", value.get("message").and_then(|value| value.as_str()).unwrap_or("unknown error")));
                }
                return Ok(value.get("result").cloned().unwrap_or(serde_json::Value::Null));
            }
        }
    }

    fn context_for(&mut self, url: &str) -> Result<Option<String>, String> {
        let tree = self.call("browsingContext.getTree", serde_json::json!({"maxDepth": 10}))?;
        let wanted_video = youtube_video_id(url);
        let mut contexts = Vec::new();
        fn collect(node: &serde_json::Value, values: &mut Vec<(String, String, Option<String>)>) {
            if let (Some(id), Some(url)) = (node.get("context").and_then(|v| v.as_str()), node.get("url").and_then(|v| v.as_str())) {
                values.push((id.to_string(), url.to_string(), node.get("clientWindow").and_then(|v| v.as_str()).map(str::to_string)));
            }
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for child in children { collect(child, values); }
            }
        }
        if let Some(roots) = tree.get("contexts").and_then(|value| value.as_array()) {
            for root in roots { collect(root, &mut contexts); }
        }
        let selected = contexts.iter().find(|(_, candidate, _)| {
            if youtube_url(url) {
                youtube_url(candidate) && wanted_video.as_ref().is_none_or(|id| youtube_video_id(candidate).as_ref() == Some(id))
            } else if twitch_url(url) {
                twitch_url(candidate) && candidate.contains("twitch.tv/")
            } else { same_website_page(url, candidate) }
        }).or_else(|| {
            if youtube_url(url) || twitch_url(url) { None }
            else { contexts.iter().find(|(_, candidate, _)| same_website_host(url, candidate)) }
        }).map(|(id, _, window)| (id.clone(), window.clone()));
        if let Some((id, window)) = selected.as_ref() {
            if self.context_id.as_deref() != Some(id.as_str()) {
                if let Some(window) = window {
                    // Firefox may ignore page actions while its native window is minimized.
                    #[cfg(not(windows))]
                    let _ = self.call("browser.setClientWindowState", serde_json::json!({"clientWindow":window,"state":"normal"}));
                }
                let _ = self.call("browsingContext.activate", serde_json::json!({"context":id}));
                if let Some(window) = window {
                    if let Err(error) = self.call("browser.setClientWindowState", serde_json::json!({"clientWindow":window,"state":"fullscreen"})) {
                        eprintln!("could not fullscreen the LibreWolf playback window: {error}");
                    }
                }
                // The browser window is fullscreened above; the page's embedded
                // video gets a separate, less persistent fullscreen attempt below.
                self.fullscreen_attempted_at = None;
            }
        }
        let selected = selected.map(|(id, _)| id);
        self.context_id = selected.clone();
        Ok(selected)
    }

    fn evaluate(&mut self, context: &str, expression: &str) -> Result<serde_json::Value, String> {
        let result = self.call("script.evaluate", serde_json::json!({
            "expression": expression, "target": {"context": context}, "awaitPromise": true, "resultOwnership": "none"
        }))?;
        if result.get("type").and_then(|value| value.as_str()) == Some("exception") {
            return Err("LibreWolf could not evaluate the active video page".into());
        }
        let remote = result.get("result").ok_or("LibreWolf returned no evaluated video state")?;
        let value = remote.get("value").and_then(|value| value.as_str()).ok_or("LibreWolf returned an unexpected video state")?;
        Ok(serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string())))
    }

    fn refresh(&mut self, url: &str) -> Result<Option<LibreWolfVideoState>, String> {
        if self.cached_at.is_some_and(|at| at.elapsed() < Duration::from_millis(150)) {
            return Ok(self.cached_video.clone());
        }
        let Some(context) = self.context_for(url)? else {
            self.cached_at = Some(std::time::Instant::now());
            self.cached_video = None;
            return Ok(None);
        };
        if !self.autoplay_attempted {
            let result = self.evaluate(&context, "(() => { const v=document.querySelector('video'); if (v && v.paused && !v.ended) v.play().catch(()=>{}); return JSON.stringify({hasVideo:!!v}); })()")?;
            self.autoplay_attempted = result.get("hasVideo").and_then(|value| value.as_bool()).unwrap_or(false);
        }
        const STATE_SCRIPT: &str = r#"(() => {
          const v=document.querySelector('video');
          const player=document.querySelector('.html5-video-player, [data-a-target="video-player"]');
          const b=document.querySelector('.ytp-fullscreen-button, [data-a-target="player-fullscreen-button"]') ||
            [...document.querySelectorAll('button,[role="button"],[aria-label],[title]')].find(e=>/full.?screen|expand/i.test((e.getAttribute('aria-label')||'')+' '+(e.getAttribute('title')||'')));
          const r=b?.getBoundingClientRect();
          const f=document.fullscreenElement;
          const embeddedFullscreen=!!f && (f===v || !!v?.contains(f) || f===player || !!player?.contains(f) || !!player?.classList.contains('ytp-fullscreen'));
          if (v && !v.__mediabridgeEndHook) {
            v.__mediabridgeEndHook=true;
            v.addEventListener('ended',()=>window.__mediabridgeVideoEnded=true);
            v.addEventListener('playing',()=>window.__mediabridgeVideoEnded=false);
          }
          return JSON.stringify({title:document.title,url:location.href,currentTime:v?.currentTime,duration:v?.duration,
            volume:v?.volume,paused:v?.paused,ended:!!v?.ended || !!window.__mediabridgeVideoEnded,embeddedFullscreen,
            hasVideo:!!v,button:r&&r.width&&r.height?{x:r.x+r.width/2,y:r.y+r.height/2}:null});
        })()"#;
        let value = self.evaluate(&context, STATE_SCRIPT)?;
        let supported_fullscreen = youtube_url(url) || twitch_url(url);
        let needs_embedded_fullscreen = value.get("embeddedFullscreen").and_then(|value| value.as_bool()) != Some(true);
        let has_fullscreen_target = value.get("button").is_some() || value.get("hasVideo").and_then(|value| value.as_bool()) == Some(true);
        let should_attempt = if supported_fullscreen {
            self.fullscreen_attempted_at.is_none_or(|attempt| attempt.elapsed() >= Duration::from_secs(2))
        } else {
            self.fullscreen_attempted_at.is_none() && has_fullscreen_target
        };
        if needs_embedded_fullscreen && should_attempt {
            // Generic sites get one attempt once a video or fullscreen control appears.
            // YouTube and Twitch retain their existing retry behavior.
            if !supported_fullscreen { self.fullscreen_attempted_at = Some(std::time::Instant::now()); }
            let attempted = if let Some(button) = value.get("button") {
                if let (Some(x), Some(y)) = (button.get("x").and_then(|value| value.as_f64()), button.get("y").and_then(|value| value.as_f64())) {
                    self.call("input.performActions", serde_json::json!({"context":context,"actions":[{"type":"pointer","id":"mediabridge-mouse","actions":[{"type":"pointerMove","x":x,"y":y,"duration":250},{"type":"pointerDown","button":0},{"type":"pointerUp","button":0}]}]})).is_ok()
                } else { false }
            } else if !supported_fullscreen {
                self.evaluate(&context, "(() => { const v=document.querySelector('video'); if (!v?.requestFullscreen) return 'unavailable'; return v.requestFullscreen().then(()=> 'ok').catch(()=> 'blocked'); })()")
                    .is_ok_and(|result| result.as_str() == Some("ok"))
            } else { false };
            if supported_fullscreen && attempted { self.fullscreen_attempted_at = Some(std::time::Instant::now()); }
        }
        let video = LibreWolfVideoState {
            title: value.get("title").and_then(|value| value.as_str()).map(str::to_string),
            current_time: value.get("currentTime").and_then(|value| value.as_f64()),
            duration: value.get("duration").and_then(|value| value.as_f64()),
            volume: value.get("volume").and_then(|value| value.as_f64()),
            paused: value.get("paused").and_then(|value| value.as_bool()),
            ended: value.get("ended").and_then(|value| value.as_bool()).unwrap_or(false),
        };
        self.cached_at = Some(std::time::Instant::now());
        self.cached_video = Some(video.clone());
        Ok(Some(video))
    }

    fn set_youtube_autoplay(&mut self, url: &str, enabled: bool) -> Result<(), String> {
        if !youtube_url(url) { return Ok(()); }
        let Some(context) = self.context_for(url)? else { return Ok(()); };
        let script = format!("(() => {{ const b=document.querySelector('.ytp-autonav-toggle-button'); if (b && b.getAttribute('aria-checked') !== '{}') b.click(); return true; }})()", if enabled { "true" } else { "false" });
        let _ = self.evaluate(&context, &script)?;
        Ok(())
    }

    fn close_playback(&mut self) -> Result<(), String> {
        if let Some(context) = self.context_id.take() {
            self.call("browsingContext.close", serde_json::json!({"context":context,"promptUnload":false}))?;
        }
        self.cached_at = None;
        self.cached_video = None;
        self.autoplay_attempted = false;
        self.fullscreen_attempted_at = None;
        Ok(())
    }

    fn close_browser(&mut self) -> Result<(), String> {
        self.call("browser.close", serde_json::json!({}))?;
        self.context_id = None;
        self.cached_at = None;
        self.cached_video = None;
        Ok(())
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn refresh_librewolf_video(state: &mut State) -> Option<LibreWolfVideoState> {
    let item = state.current.as_ref().filter(|item| !item.local_playback
        && item.browser.as_deref().is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf")))?;
    let url = item.url.clone();
    if state.librewolf_bidi.is_none() {
        if state.librewolf_bidi_retry_at.is_some_and(|retry| retry > std::time::Instant::now()) { return None; }
        state.librewolf_bidi_retry_at = Some(std::time::Instant::now() + Duration::from_secs(3));
        match librewolf_bidi_session() {
            Ok(session) => { state.librewolf_bidi = Some(session); state.librewolf_bidi_retry_at = None; }
            Err(error) => { eprintln!("LibreWolf playback controls are unavailable: {error}"); return None; }
        }
    }
    match state.librewolf_bidi.as_mut().map(|bidi| bidi.refresh(&url)) {
        Some(Ok(result)) => result,
        Some(Err(error)) => {
            eprintln!("LibreWolf playback controls stopped: {error}");
            state.librewolf_bidi = None;
            state.librewolf_bidi_retry_at = Some(std::time::Instant::now() + Duration::from_secs(3));
            None
        }
        None => None,
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn control_librewolf(state: &mut State, operation: &str, value: Option<f64>) -> Result<(), String> {
    if refresh_librewolf_video(state).is_none() {
        return Err("could not find the active LibreWolf video tab; make sure playback uses MediaBridge LibreWolf".into());
    }
    let bidi = state.librewolf_bidi.as_mut().ok_or("LibreWolf BiDi session is not active")?;
    let context = bidi.context_id.clone().ok_or("could not identify the LibreWolf playback tab")?;
    if matches!(operation, "zoom_in" | "zoom_out" | "zoom_reset") {
        let mut actions = vec![serde_json::json!({"type":"keyDown","value":"\u{e009}"})];
        match operation {
            "zoom_in" => {
                actions.push(serde_json::json!({"type":"keyDown","value":"\u{e008}"}));
                actions.push(serde_json::json!({"type":"keyDown","value":"="}));
                actions.push(serde_json::json!({"type":"keyUp","value":"="}));
                actions.push(serde_json::json!({"type":"keyUp","value":"\u{e008}"}));
            }
            "zoom_out" => {
                actions.push(serde_json::json!({"type":"keyDown","value":"-"}));
                actions.push(serde_json::json!({"type":"keyUp","value":"-"}));
            }
            _ => {
                actions.push(serde_json::json!({"type":"keyDown","value":"0"}));
                actions.push(serde_json::json!({"type":"keyUp","value":"0"}));
            }
        }
        actions.push(serde_json::json!({"type":"keyUp","value":"\u{e009}"}));
        bidi.call("input.performActions", serde_json::json!({
            "context": context,
            "actions": [{"type":"key","id":"mediabridge-page-zoom","actions":actions}]
        }))?;
        bidi.cached_at = None;
        return Ok(());
    }
    let video_state = bidi.evaluate(&context, "JSON.stringify({hasVideo:!!document.querySelector('video')})")?;
    if video_state.get("hasVideo").and_then(|value| value.as_bool()) != Some(true) {
        return Err("the active LibreWolf page does not expose a controllable HTML video".into());
    }
    let expression = match operation {
        "pause" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.pause(); return 'ok'; })()".to_string(),
        "resume" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.play(); return 'ok'; })()".to_string(),
        "play_pause" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.paused?v.play():v.pause(); return 'ok'; })()".to_string(),
        "rewind" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.currentTime=Math.max(0,v.currentTime-5); return 'ok'; })()".to_string(),
        "forward" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.currentTime=Math.min(v.duration||Infinity,v.currentTime+5); return 'ok'; })()".to_string(),
        "seek" => format!("(() => {{ const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.currentTime={}; return 'ok'; }})()", value.ok_or("seek position is missing")?),
        "volume" => format!("(() => {{ const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.volume={}; return 'ok'; }})()", value.ok_or("volume is missing")?.clamp(0.0, 1.0)),
        "volume_up" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.volume=Math.min(1,v.volume+0.05); return 'ok'; })()".to_string(),
        "volume_down" => "(() => { const v=document.querySelector('video'); if (!v) throw Error('video unavailable'); v.volume=Math.max(0,v.volume-0.05); return 'ok'; })()".to_string(),
        _ => return Err(format!("invalid browser control operation '{operation}'")),
    };
    let _ = bidi.evaluate(&context, &expression)?;
    bidi.cached_at = None;
    Ok(())
}

fn cdp_call(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: u64, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let payload = serde_json::json!({"id": id, "method": method, "params": params}).to_string();
    socket.send(Message::Text(payload)).map_err(|e| format!("could not send Chromium DevTools command: {e}"))?;
    loop {
        let message = socket.read().map_err(|e| format!("could not read Chromium DevTools response: {e}"))?;
        if let Message::Text(text) = message {
            let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("invalid Chromium DevTools message: {e}"))?;
            if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(error) = value.get("error") { return Err(format!("Chromium DevTools rejected {method}: {error}")); }
                return Ok(value.get("result").cloned().unwrap_or(serde_json::Value::Null));
            }
        }
    }
}

fn loopback_websocket_url(url: &str, port: u16) -> Result<String, String> {
    let (_, authority_and_path) = url.split_once("://").ok_or_else(|| format!("invalid Chromium DevTools WebSocket URL: {url}"))?;
    let path = authority_and_path.find('/').map(|i| &authority_and_path[i..])
        .ok_or_else(|| format!("Chromium DevTools WebSocket URL has no path: {url}"))?;
    Ok(format!("ws://127.0.0.1:{port}{path}"))
}

const YOUTUBE_PLAYER_STATE: &str = r#"(() => {
  const video = document.querySelector('video');
  const player = document.querySelector('.html5-video-player');
  const box = (element) => {
    if (!element) return null;
    const r = element.getBoundingClientRect();
    const s = getComputedStyle(element);
    if (!r.width || !r.height || s.display === 'none' || s.visibility === 'hidden') return null;
    return {x:r.x+r.width/2,y:r.y+r.height/2,left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:r.width,height:r.height};
  };
  const play = document.querySelector('.ytp-large-play-button') || document.querySelector('.ytp-play-button[aria-label*="Play"]');
  const fullscreen = document.querySelector('.ytp-fullscreen-button');
  const pr = box(player), pb = box(play), fb = box(fullscreen);
  const inside = (b) => b && pr && b.left >= pr.left && b.right <= pr.right && b.top >= pr.top && b.bottom <= pr.bottom;
  return {host:location.hostname,path:location.pathname,url:location.href,ready:video?.readyState ?? -1,
    paused:video?.paused ?? true,duration:video?.duration ?? 0,
    player:pr,play:inside(pb)?pb:null,fullscreen:inside(fb)?fb:null,
    isFullscreen:!!document.fullscreenElement || !!player?.classList.contains('ytp-fullscreen')};
})()"#;

fn cdp_value(result: &serde_json::Value) -> Result<serde_json::Value, String> {
    result.get("result").and_then(|r| r.get("value")).cloned()
        .ok_or_else(|| "Chromium DevTools did not return page state".into())
}

fn youtube_state(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: &mut u64) -> Result<serde_json::Value, String> {
    *id += 1;
    let result = cdp_call(socket, *id, "Runtime.evaluate", serde_json::json!({
        "expression": YOUTUBE_PLAYER_STATE, "returnByValue": true, "awaitPromise": true, "userGesture": true
    }))?;
    cdp_value(&result)
}

fn click_youtube_control(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: &mut u64, point: &serde_json::Value) -> Result<(), String> {
    let x = point.get("x").and_then(|v| v.as_f64()).ok_or("YouTube control has no x coordinate")?;
    let y = point.get("y").and_then(|v| v.as_f64()).ok_or("YouTube control has no y coordinate")?;
    *id += 1;
    cdp_call(socket, *id, "Input.dispatchMouseEvent", serde_json::json!({"type":"mouseMoved","x":x,"y":y}))?;
    thread::sleep(Duration::from_millis(250));
    *id += 1;
    cdp_call(socket, *id, "Input.dispatchMouseEvent", serde_json::json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1}))?;
    *id += 1;
    cdp_call(socket, *id, "Input.dispatchMouseEvent", serde_json::json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1}))?;
    Ok(())
}

fn press_youtube_fullscreen_shortcut(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: &mut u64) -> Result<(), String> {
    *id += 1;
    cdp_call(socket, *id, "Page.bringToFront", serde_json::json!({}))?;
    let state = youtube_state(socket, id)?;
    let player = state.get("player").filter(|p| !p.is_null()).ok_or("YouTube player bounds disappeared before fullscreen shortcut")?;
    let x = player.get("x").and_then(|v| v.as_f64()).ok_or("YouTube player has no x coordinate")?;
    let y = player.get("y").and_then(|v| v.as_f64()).ok_or("YouTube player has no y coordinate")?;
    *id += 1;
    cdp_call(socket, *id, "Input.dispatchMouseEvent", serde_json::json!({"type":"mouseMoved","x":x,"y":y}))?;
    // Let the browser activate the page and the player controls before sending
    // the desktop key. F is only handled by YouTube while its player is focused.
    thread::sleep(Duration::from_millis(800));
    send_desktop_shortcut("youtube-f")
}

fn automate_youtube(profile: &Path) -> Result<(), String> {
    let port = chromium_debug_port(profile)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if local_http_json(port, "/json/version").is_ok() { break; }
        if std::time::Instant::now() >= deadline { return Err("timed out waiting for Chromium DevTools to start".into()); }
        thread::sleep(Duration::from_millis(200));
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    let (ws_url, page_url) = loop {
        let targets = local_http_json(port, "/json/list")?;
        if let Some(page) = targets.as_array().and_then(|pages| pages.iter().find(|p| {
            p.get("type").and_then(|v| v.as_str()) == Some("page") &&
            p.get("url").and_then(|v| v.as_str()).is_some_and(youtube_url)
        })) {
            if let (Some(ws), Some(url)) = (page.get("webSocketDebuggerUrl").and_then(|v| v.as_str()), page.get("url").and_then(|v| v.as_str())) {
                break (ws.to_string(), url.to_string());
            }
        }
        if std::time::Instant::now() >= deadline { return Err("timed out waiting for the YouTube video page to load".into()); }
        thread::sleep(Duration::from_millis(250));
    };
    let ws_url = loopback_websocket_url(&ws_url, port)?;
    println!("Using YouTube DevTools endpoint {ws_url}");
    let (mut socket, _) = connect(&ws_url).map_err(|e| format!("could not connect to YouTube DevTools endpoint {ws_url}: {e}"))?;
    let mut id = 0;
    id += 1;
    cdp_call(&mut socket, id, "Runtime.enable", serde_json::json!({}))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    let mut state = loop {
        let state = youtube_state(&mut socket, &mut id)?;
        let host = state.get("host").and_then(|v| v.as_str()).unwrap_or("");
        let path = state.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if !(host == "youtube.com" || host.ends_with(".youtube.com")) || !(path == "/watch" || path.starts_with("/shorts/") || path.starts_with("/live/")) {
            return Err(format!("page did not resolve to a YouTube video: {}", state.get("url").and_then(|v| v.as_str()).unwrap_or(&page_url)));
        }
        if state.get("ready").and_then(|v| v.as_i64()).unwrap_or(-1) >= 2 { break state; }
        if std::time::Instant::now() >= deadline { return Err("YouTube page loaded, but its video did not become ready".into()); }
        thread::sleep(Duration::from_millis(400));
    };
    println!("Verified YouTube video page and ready video: {}", state.get("url").and_then(|v| v.as_str()).unwrap_or(&page_url));
    if state.get("paused").and_then(|v| v.as_bool()).unwrap_or(true) {
        let point = state.get("play").filter(|p| !p.is_null()).ok_or("video is paused, but no visible YouTube play control was found; leaving the page untouched")?.clone();
        click_youtube_control(&mut socket, &mut id, &point)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            state = youtube_state(&mut socket, &mut id)?;
            if !state.get("paused").and_then(|v| v.as_bool()).unwrap_or(true) { break; }
            if std::time::Instant::now() >= deadline { return Err("clicked YouTube's visible play control but could not confirm playback".into()); }
            thread::sleep(Duration::from_millis(200));
        }
        println!("Confirmed YouTube video is playing");
    } else {
        println!("YouTube video was already playing");
    }
    if !state.get("isFullscreen").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("Trying YouTube's F fullscreen shortcut");
        press_youtube_fullscreen_shortcut(&mut socket, &mut id)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut fullscreen = false;
        while std::time::Instant::now() < deadline {
            state = youtube_state(&mut socket, &mut id)?;
            if state.get("isFullscreen").and_then(|v| v.as_bool()).unwrap_or(false) { fullscreen = true; break; }
            thread::sleep(Duration::from_millis(200));
        }
        if !fullscreen {
            let point = state.get("fullscreen").filter(|p| !p.is_null()).ok_or("YouTube F shortcut did not enter fullscreen and no visible fullscreen button was found")?.clone();
            click_youtube_control(&mut socket, &mut id, &point)?;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                state = youtube_state(&mut socket, &mut id)?;
                if state.get("isFullscreen").and_then(|v| v.as_bool()).unwrap_or(false) { break; }
                if std::time::Instant::now() >= deadline { return Err("YouTube fullscreen shortcut and control did not enter video fullscreen".into()); }
                thread::sleep(Duration::from_millis(200));
            }
        }
        println!("Confirmed YouTube video is fullscreen");
    } else {
        println!("YouTube video was already fullscreen");
    }
    Ok(())
}

const YOUTUBE_GUEST_CONTROL: &str = r#"(() => {
  const norm = (s) => (s || '').replace(/\s+/g, ' ').trim().toLowerCase();
  const visible = (el) => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden' &&
      Number(s.opacity) > 0 && s.pointerEvents !== 'none' && r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight;
  };
  const guest = Array.from(document.querySelectorAll('button,[role="button"],a,[tabindex]'))
    .find(el => visible(el) && norm(el.innerText || el.textContent || el.getAttribute('aria-label')) === 'watch as guest');
  if (!guest) return {host:location.hostname,path:location.pathname,hash:location.hash,guest:null};
  const r = guest.getBoundingClientRect();
  return {host:location.hostname,path:location.pathname,hash:location.hash,
    guest:{x:r.x+r.width/2,y:r.y+r.height/2,label:(guest.innerText || guest.textContent || '').trim()}};
})()"#;

fn youtube_guest_state(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: &mut u64) -> Result<serde_json::Value, String> {
    *id += 1;
    let result = cdp_call(socket, *id, "Runtime.evaluate", serde_json::json!({
        "expression": YOUTUBE_GUEST_CONTROL, "returnByValue": true, "awaitPromise": true
    }))?;
    cdp_value(&result)
}

fn automate_youtube_guest(profile: &Path) -> Result<(), String> {
    let port = chromium_debug_port(profile)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if local_http_json(port, "/json/version").is_ok() { break; }
        if std::time::Instant::now() >= deadline { return Err("timed out waiting for Chromium DevTools to start".into()); }
        thread::sleep(Duration::from_millis(200));
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let (ws_url, page_url) = loop {
        let targets = local_http_json(port, "/json/list")?;
        if let Some(page) = targets.as_array().and_then(|pages| pages.iter().find(|p| {
            p.get("type").and_then(|v| v.as_str()) == Some("page") &&
            p.get("url").and_then(|v| v.as_str()).is_some_and(youtube_url) &&
            p.get("url").and_then(|v| v.as_str()).is_some_and(youtube_tv_url)
        })) {
            if let (Some(ws), Some(url)) = (page.get("webSocketDebuggerUrl").and_then(|v| v.as_str()), page.get("url").and_then(|v| v.as_str())) {
                break (ws.to_string(), url.to_string());
            }
        }
        if std::time::Instant::now() >= deadline { return Err("timed out waiting for the YouTube TV page".into()); }
        thread::sleep(Duration::from_millis(250));
    };
    let ws_url = loopback_websocket_url(&ws_url, port)?;
    println!("Using YouTube TV DevTools endpoint {ws_url}");
    let (mut socket, _) = connect(&ws_url).map_err(|e| format!("could not connect to YouTube TV DevTools endpoint {ws_url}: {e}"))?;
    let mut id = 0;
    id += 1;
    cdp_call(&mut socket, id, "Runtime.enable", serde_json::json!({}))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut state;
    loop {
        state = youtube_guest_state(&mut socket, &mut id)?;
        let host = state.get("host").and_then(|v| v.as_str()).unwrap_or("");
        if !(host == "youtube.com" || host.ends_with(".youtube.com")) || state.get("path").and_then(|v| v.as_str()) != Some("/tv") {
            return Err(format!("active page is not YouTube TV: {}", state.get("path").and_then(|v| v.as_str()).unwrap_or("unknown path")));
        }
        if state.get("guest").is_some_and(|g| !g.is_null()) { break; }
        if std::time::Instant::now() >= deadline {
            println!("No visible 'Watch as guest' control on YouTube TV; no input was sent (page: {page_url})");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(300));
    }
    let guest = state.get("guest").cloned().ok_or("guest control disappeared before selection")?;
    println!("Found visible YouTube TV control '{}'; selecting guest playback", guest.get("label").and_then(|v| v.as_str()).unwrap_or("Watch as guest"));
    click_youtube_control(&mut socket, &mut id, &guest)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        state = youtube_guest_state(&mut socket, &mut id)?;
        if state.get("guest").map_or(true, serde_json::Value::is_null) {
            println!("YouTube TV guest selection completed");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline { return Err("clicked the visible guest control but it remained on screen".into()); }
        thread::sleep(Duration::from_millis(250));
    }
}

const TWITCH_THEATER_CONTROL: &str = r#"(() => {
  const visible = (el) => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity) > 0 && r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight;
  };
  const label = (el) => (el.getAttribute('aria-label') || el.getAttribute('title') || el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
  const controls = Array.from(document.querySelectorAll('button,[role="button"]')).filter(visible);
  const theaterControls = controls.filter(el => /theat(?:er|re) mode/i.test(label(el)));
  const active = theaterControls.find(el => /(?:exit|leave).*theat(?:er|re)|theat(?:er|re).*on/i.test(label(el)) || el.getAttribute('aria-pressed') === 'true' || el.getAttribute('aria-checked') === 'true');
  const button = theaterControls.find(el => !/(?:exit|leave)/i.test(label(el)));
  const videoReady = document.querySelector('video')?.readyState ?? -1;
  if (active) return {host:location.hostname,path:location.pathname,active:true,label:label(active),videoReady};
  return {host:location.hostname,path:location.pathname,active:false,label:button ? label(button) : null,videoReady};
})()"#;

fn press_twitch_theater_shortcut() -> Result<(), String> {
    send_desktop_shortcut("twitch-alt-t")
}

fn twitch_theater_state(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, id: &mut u64) -> Result<serde_json::Value, String> {
    *id += 1;
    let result = cdp_call(socket, *id, "Runtime.evaluate", serde_json::json!({
        "expression": TWITCH_THEATER_CONTROL, "returnByValue": true, "awaitPromise": true
    }))?;
    cdp_value(&result)
}

fn automate_twitch_theater(profile: &Path) -> Result<(), String> {
    let port = chromium_debug_port(profile)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if local_http_json(port, "/json/version").is_ok() { break; }
        if std::time::Instant::now() >= deadline { return Err("timed out waiting for Chromium DevTools to start".into()); }
        thread::sleep(Duration::from_millis(200));
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    let (ws_url, page_url) = loop {
        let targets = local_http_json(port, "/json/list")?;
        if let Some(page) = targets.as_array().and_then(|pages| pages.iter().find(|p| {
            p.get("type").and_then(|v| v.as_str()) == Some("page") &&
            p.get("url").and_then(|v| v.as_str()).is_some_and(twitch_url)
        })) {
            if let (Some(ws), Some(url)) = (page.get("webSocketDebuggerUrl").and_then(|v| v.as_str()), page.get("url").and_then(|v| v.as_str())) {
                break (ws.to_string(), url.to_string());
            }
        }
        if std::time::Instant::now() >= deadline { return Err("timed out waiting for the Twitch channel page to load".into()); }
        thread::sleep(Duration::from_millis(250));
    };
    let ws_url = loopback_websocket_url(&ws_url, port)?;
    println!("Using Twitch DevTools endpoint {ws_url}");
    let (mut socket, _) = connect(&ws_url).map_err(|e| format!("could not connect to Twitch DevTools endpoint {ws_url}: {e}"))?;
    let mut id = 0;
    id += 1;
    cdp_call(&mut socket, id, "Runtime.enable", serde_json::json!({}))?;
    id += 1;
    cdp_call(&mut socket, id, "Page.bringToFront", serde_json::json!({}))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(40);
    let mut state;
    loop {
        state = twitch_theater_state(&mut socket, &mut id)?;
        let host = state.get("host").and_then(|v| v.as_str()).unwrap_or("");
        if !(host == "twitch.tv" || host.ends_with(".twitch.tv")) {
            return Err(format!("active page is not Twitch: {}", state.get("host").and_then(|v| v.as_str()).unwrap_or("unknown host")));
        }
        if state.get("active").and_then(|v| v.as_bool()) == Some(true) {
            println!("Twitch is already in theater mode");
            return Ok(());
        }
        if state.get("videoReady").and_then(|v| v.as_i64()).unwrap_or(-1) >= 2 { break; }
        if std::time::Instant::now() >= deadline {
            return Err(format!("Twitch page loaded but the stream player was not ready (page: {page_url})"));
        }
        thread::sleep(Duration::from_millis(400));
    }
    // Hover the actual Twitch player, then give the page time to finish its
    // initial layout before sending Alt+T to the foreground desktop window.
    id += 1;
    let player = cdp_call(&mut socket, id, "Runtime.evaluate", serde_json::json!({
        "expression": "(() => { const r=document.querySelector('video')?.getBoundingClientRect(); return r ? {x:r.x+r.width/2,y:r.y+r.height/2} : null })()",
        "returnByValue": true
    }))?;
    let point = cdp_value(&player)?;
    if let (Some(x), Some(y)) = (point.get("x").and_then(|v| v.as_f64()), point.get("y").and_then(|v| v.as_f64())) {
        id += 1;
        cdp_call(&mut socket, id, "Input.dispatchMouseEvent", serde_json::json!({"type":"mouseMoved","x":x,"y":y}))?;
    }
    println!("Twitch stream is ready; waiting briefly for player controls to settle");
    thread::sleep(Duration::from_millis(1200));
    println!("Sending desktop Alt+T for Twitch theater mode");
    press_twitch_theater_shortcut()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        state = twitch_theater_state(&mut socket, &mut id)?;
        if state.get("active").and_then(|v| v.as_bool()) == Some(true) {
            println!("Confirmed Twitch theater mode");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            println!("Sent Twitch Alt+T, but could not verify theater mode from the player control state");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(300));
    }
}

fn playback_error_from_line(line: &str) -> Option<&'static str> {
    let line = line.to_ascii_lowercase();
    if line.contains("sign in to confirm") || line.contains("confirm you're not a bot") || line.contains("confirm you’re not a bot") {
        Some("YouTube asked yt-dlp to confirm you are not a bot. Set up local browser cookies in MediaBridge, then try this item again.")
    } else if line.contains("http error 403") || line.contains("403 forbidden") {
        Some("The media host rejected mpv's stream request (HTTP 403). This can be temporary; YouTube browser cookies may help. See the server log for details.")
    } else {
        None
    }
}

fn watch_mpv_stderr(stderr: std::process::ChildStderr, epoch: u64, error_tx: Sender<(u64, String)>) {
    thread::spawn(move || {
        let mut reported = false;
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break; };
            eprintln!("{line}");
            if !reported {
                if let Some(message) = playback_error_from_line(&line) {
                    let _ = error_tx.send((epoch, message.into()));
                    reported = true;
                }
            }
        }
    });
}

fn drain_player_errors(state: &mut State) {
    let messages: Vec<(u64, String)> = state.player_error_rx.as_ref()
        .map(|receiver| receiver.try_iter().collect()).unwrap_or_default();
    for (epoch, message) in messages {
        if epoch == state.playback_epoch {
            eprintln!("MediaBridge playback warning: {message}");
            notify_desktop("Media could not be played", &message);
            state.playback_error = Some(message);
        }
    }
}

fn notify_desktop(title: &str, body: &str) {
    let Ok(mut child) = Command::new("notify-send")
        .args(["--app-name=MediaBridge", "--urgency=critical", title, body])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
            return;
        };
    thread::spawn(move || { let _ = child.wait(); });
}

fn start_url(state: &mut State, cfg: &Config, mut item: QueueItem) -> Result<(), String> {
    state.autoplay_waiting_to_advance = false;
    let preferences = current_playback_preferences(cfg);
    if !item.local_playback { set_youtube_interface(&mut item)?; }
    let is_youtube = youtube_url(&item.url);
    let is_youtube_tv = youtube_tv_url(&item.url);
    let youtube_guest = item.youtube_guest;
    let twitch_theater = item.twitch_theater && twitch_url(&item.url);
    let mut automation_profile = None;
    let mut mpv_socket = None;
    let mut child = if item.local_playback {
        #[cfg(not(any(unix, windows)))]
        return Err("local mpv playback is not supported on this platform".into());
        #[cfg(any(unix, windows))]
        {
            let socket = mpv_socket_path();
            let _ = std::fs::remove_file(&socket);
            let mut args = vec![
                "--fullscreen".to_string(), "--force-window=yes".to_string(),
                // mpv.conf can request a minimized window; force it visible for
                // links opened by the background tray server.
                "--window-minimized=no".to_string(),
                format!("--input-ipc-server={}", socket.display()),
                "--title=MediaBridge".to_string(), format!("--volume={}", state.last_mpv_volume.map(|v| (v.clamp(0.0, 1.0) * 100.0).round() as u8).unwrap_or(100)), "--volume-max=100".to_string(),
                "--script-opts=ytdl_hook-ytdl_path=yt-dlp".to_string(),
            ];
            if item.url.get(..6).is_some_and(|scheme| scheme.eq_ignore_ascii_case("smb://")) { args.push("--ytdl=no".into()); }
            let cookie_browser = match cookie_browser_spec(&preferences.cookies_browser, preferences.cookies_profile.as_deref()) {
                Ok(browser) => browser,
                Err(error) => { eprintln!("could not select the default cookie browser; continuing without browser cookies: {error}"); None }
            };
            if let Some(browser) = cookie_browser.as_deref().filter(|browser| !browser.trim().is_empty()) {
                if browser.contains(',') || browser.contains('\n') || browser.contains('\r') {
                    return Err("cookie browser setting must not contain commas or line breaks".into());
                }
                println!("Using local yt-dlp cookies from browser profile '{browser}' for mpv playback");
                let mut raw_options = format!("remote-components=ejs:github,cookies-from-browser={browser}");
                let subtitles = item.subtitles.unwrap_or(preferences.subtitles);
                if !subtitles { raw_options.push_str(",sub-langs=none"); }
                else if let Some(language) = preferences.preferred_language.as_deref().filter(|v| !v.trim().is_empty()) {
                    raw_options.push_str(&format!(",sub-langs={}", language.trim()));
                }
                args.push(format!("--ytdl-raw-options={raw_options}"));
            } else {
                let subtitles = item.subtitles.unwrap_or(preferences.subtitles);
                let raw_options = if subtitles {
                    if let Some(language) = preferences.preferred_language.as_deref().filter(|v| !v.trim().is_empty()) {
                        format!("remote-components=ejs:github,sub-langs={}", language.trim())
                    } else { "remote-components=ejs:github".to_string() }
                } else {
                    "remote-components=ejs:github,sub-langs=none".to_string()
                };
                args.push(format!("--ytdl-raw-options={raw_options}"));
            }
            if let Some(quality) = item.quality.as_deref().or(preferences.default_quality.as_deref()) {
                if !quality.eq_ignore_ascii_case("best") {
                    let height = quality.parse::<u16>().ok().filter(|h| (144..=4320).contains(h))
                        .ok_or("quality must be 'best' or a height from 144 to 4320")?;
                    args.push(format!("--ytdl-format=bv*[height<={height}]+ba/b[height<={height}]"));
                }
            }
            if let Some(screen) = item.screen.or(preferences.default_screen) {
                args.push(format!("--fs-screen={screen}"));
            }
            args.push(item.url.clone());
            let mut command = media_tool_command("mpv");
            command.args(args).stdin(Stdio::null()).stderr(Stdio::piped());
            add_deno_to_path(&mut command);
            isolate_process_group(&mut command);
            let mut child = command.spawn()
                .map_err(|e| format!("could not start mpv; install mpv and yt-dlp: {e}"))?;
            #[cfg(windows)]
            {
                let mpv_process_id = child.id();
                thread::spawn(move || raise_mpv_window(mpv_process_id));
            }
            if let (Some(stderr), Some(error_tx)) = (child.stderr.take(), state.player_error_tx.clone()) {
                watch_mpv_stderr(stderr, state.playback_epoch, error_tx);
            }
            println!("Launched mpv for {}", safe_media_url(&item.url));
            mpv_socket = Some(socket);
            Some(child)
        }
    } else if let Some(browser) = item.browser.clone() {
        let defaults = if item.browser_args.is_empty() && is_youtube_tv && chromium_name(&browser) {
            vec![
                "--app={url}".into(),
                "--kiosk".into(),
                "--autoplay-policy=no-user-gesture-required".into(),
                "--user-agent=Mozilla/5.0 (PS4; Leanback Shell) Cobalt/26.lts.0-qa; compatible;".into(),
            ]
        } else if item.browser_args.is_empty() {
            let name = Path::new(&browser).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_else(|| browser.to_lowercase());
            match name.as_str() {
                "chromium" | "chromium-browser" | "google-chrome" | "google-chrome-stable" => vec![
                    "--kiosk".into(),
                    "--start-fullscreen".into(),
                    "--new-window".into(),
                    "--autoplay-policy=no-user-gesture-required".into(),
                ],
                "firefox" | "librewolf" | "mediabridge-librewolf" => vec!["--new-window".into()],
                _ => Vec::new(),
            }
        } else { Vec::new() };
        let template = if item.browser_args.is_empty() { defaults } else { item.browser_args.clone() };
        let mut args = expand(&template, &item.url, "{url}");
        if !template.iter().any(|a| a.contains("{url}")) { args.push(item.url.clone()); }
        let mut launched = None;
        for candidate in browser_candidates(&browser) {
            let profile = if (youtube_url(&item.url) || twitch_theater) && chromium_name(&candidate) {
                Some(chromium_profile()?)
            } else { None };
            let mut launch_args = args.clone();
            #[cfg(windows)]
            if candidate.to_ascii_lowercase().ends_with("librewolf.exe") {
                let requested_url = launch_args.iter().position(|argument| argument == &item.url)
                    .map(|index| launch_args.remove(index));
                let profile = managed_librewolf_profile()
                    .ok_or("could not locate the persistent MediaBridge LibreWolf profile")?;
                ensure_librewolf_profile(&profile)?;
                launch_args.push("--profile".into());
                launch_args.push(profile.to_string_lossy().into_owned());
                launch_args.push("--remote-debugging-port".into());
                launch_args.push("9222".into());
                if let Some(url) = requested_url { launch_args.push(url); }
            }
            if let Some(profile) = profile.as_ref() {
                launch_args.push(format!("--user-data-dir={}", profile.display()));
                launch_args.push("--remote-debugging-address=127.0.0.1".into());
                let port = chromium_debug_port(profile)?;
                launch_args.push(format!("--remote-debugging-port={port}"));
                launch_args.push("--no-first-run".into());
                launch_args.push("--no-default-browser-check".into());
            }
            let mut command = Command::new(&candidate);
            command.args(&launch_args);
            isolate_process_group(&mut command);
            match command.spawn() {
                Ok(child) => {
                    #[cfg(windows)]
                    if candidate.to_ascii_lowercase().ends_with("librewolf.exe") {
                        let process_id = child.id();
                        let display = item.screen.or(preferences.default_screen).map(usize::from);
                        thread::spawn(move || restore_librewolf_window(process_id, display));
                    }
                    println!("Launched browser '{}' for {}", candidate, safe_media_url(&item.url));
                    if let Some(profile) = profile.as_ref() {
                        println!("Waiting for Chromium DevTools on 127.0.0.1:{}", chromium_debug_port(profile)?);
                    }
                    item.browser = Some(candidate);
                    automation_profile = profile;
                    launched = Some(child);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("could not start browser '{}': {error}", candidate)),
            }
        }
        if launched.is_none() {
            eprintln!("browser '{}' was not found (tried {:?}); using the OS default browser", browser, browser_candidates(&browser));
            item.browser = None;
        }
        launched
    } else { None };
    if child.is_none() {
        let mut opener = open_url_default(&item.url)?;
        println!("Opened {} with the OS default browser", safe_media_url(&item.url));
        thread::spawn(move || match opener.wait() {
            Ok(status) if status.success() => println!("default browser launcher completed successfully"),
            Ok(status) => eprintln!("default browser launcher exited with {status}"),
            Err(error) => eprintln!("could not wait for the default browser launcher: {error}"),
        });
    }
    #[cfg(any(windows, target_os = "linux"))]
    let player_child = if item.browser.as_deref().is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf")) {
        // LibreWolf forwards URLs to its existing single-instance process and
        // exits. That short-lived launcher is not the lifetime of playback.
        drop(child.take());
        None
    } else { child.take() };
    #[cfg(not(any(windows, target_os = "linux")))]
    let player_child = child.take();
    state.player = player_child;
    state.mpv_socket = mpv_socket;
    state.current = Some(item);
    if let Some(profile) = automation_profile {
        if is_youtube_tv && youtube_guest {
            println!("YouTube TV guest fallback is enabled; waiting for its visible guest control");
            thread::spawn(move || if let Err(error) = automate_youtube_guest(&profile) {
                eprintln!("YouTube TV guest selection stopped safely: {error}");
            });
        } else if is_youtube && !is_youtube_tv {
            println!("Starting YouTube page verification and player controls");
            thread::spawn(move || if let Err(error) = automate_youtube(&profile) {
                eprintln!("YouTube automation stopped safely: {error}");
            });
        } else if is_youtube_tv {
            println!("Opened experimental YouTube TV/Leanback interface; guest selection is disabled (set youtube_guest=true to enable it)");
        } else if twitch_theater {
            println!("Starting Twitch theater-mode verification");
            thread::spawn(move || if let Err(error) = automate_twitch_theater(&profile) {
                eprintln!("Twitch theater automation stopped safely: {error}");
            });
        }
    } else if is_youtube_tv {
        println!("Opened the experimental YouTube TV/Leanback interface; standard-page automation is skipped");
    } else if twitch_theater {
        println!("Twitch theater automation requires Chromium to be selected as the browser");
    }
    Ok(())
}

fn advance(state: &mut State, cfg: &Config) {
    drain_player_errors(state);
    drain_metadata(state);
    #[cfg(any(windows, target_os = "linux"))]
    {
        if state.autoplay_until.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            set_autoplay(state, false, None);
        }
        let ended = refresh_librewolf_video(state).is_some_and(|video| video.ended);
        if let (Some(item), Some(bidi)) = (state.current.as_ref(), state.librewolf_bidi.as_mut()) {
            let _ = bidi.set_youtube_autoplay(&item.url, false);
        }
        if ended {
            if let Some(item) = state.queue.pop_front() {
                if let Some(current) = state.current.take() { push_history(state, current); }
                if let Some(bidi) = state.librewolf_bidi.as_mut() { let _ = bidi.close_playback(); }
                if let Err(error) = start_url(state, cfg, item.clone()) {
                    eprintln!("failed to start queued URL {}: {error}", safe_media_url(&item.url));
                }
            } else if state.autoplay_enabled {
                if let Some(seed) = state.current.as_ref().filter(|item| youtube_url(&item.url)).map(|item| item.url.clone()) {
                    request_autoplay_mix(state, seed);
                }
                if state.autoplay_in_flight.is_some() { state.autoplay_waiting_to_advance = true; }
                else {
                    if let Some(bidi) = state.librewolf_bidi.as_mut() { let _ = bidi.close_playback(); }
                    if let Some(item) = state.current.take() { push_history(state, item); }
                }
            } else {
                if let Some(bidi) = state.librewolf_bidi.as_mut() { let _ = bidi.close_playback(); }
                if let Some(item) = state.current.take() { push_history(state, item); }
            }
        }
    }
    loop {
        let exit = match state.player.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => Some(Ok(status)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            },
            None => break,
        };
        let Some(exit) = exit else { break; };
        match exit {
            Ok(status) if status.success() => println!("Playback process exited successfully"),
            Ok(status) => {
                let player = if state.current.as_ref().is_some_and(|item| item.local_playback) { "mpv" } else { "browser" };
                let message = format!("{player} playback process exited with {status}");
                eprintln!("{message}");
                state.playback_error = Some(message);
            }
            Err(error) => {
                let message = format!("could not read playback process status: {error}");
                eprintln!("{message}");
                state.playback_error = Some(message);
            }
        }
        state.player = None;
        if let Some(socket) = state.mpv_socket.take() { let _ = std::fs::remove_file(socket); }
        if state.queue.is_empty() && state.autoplay_enabled {
            if let Some(seed) = state.current.as_ref().filter(|item| youtube_url(&item.url)).map(|item| item.url.clone()) {
                request_autoplay_mix(state, seed);
            }
            if state.autoplay_in_flight.is_some() {
                state.autoplay_waiting_to_advance = true;
                break;
            }
        }
        if let Some(item) = state.current.take() { push_history(state, item); }
        match state.queue.pop_front() {
            Some(item) => if let Err(error) = start_url(state, cfg, item.clone()) { eprintln!("failed to start queued URL {}: {error}", safe_media_url(&item.url)); },
            None => break,
        }
    }
    drain_autoplay_results(state);
    dispatch_autoplay_mix(state);
    if state.autoplay_waiting_to_advance {
        if let Some(item) = state.queue.pop_front() {
            if let Some(current) = state.current.take() { push_history(state, current); }
            state.autoplay_waiting_to_advance = false;
            #[cfg(any(windows, target_os = "linux"))]
            if let Some(bidi) = state.librewolf_bidi.as_mut() { let _ = bidi.close_playback(); }
            if let Err(error) = start_url(state, cfg, item.clone()) { eprintln!("failed to start autoplay video {}: {error}", safe_media_url(&item.url)); }
        } else if state.autoplay_in_flight.is_none() || !state.autoplay_enabled {
            #[cfg(any(windows, target_os = "linux"))]
            if let Some(bidi) = state.librewolf_bidi.as_mut() { let _ = bidi.close_playback(); }
            if let Some(current) = state.current.take() { push_history(state, current); }
            state.autoplay_waiting_to_advance = false;
        }
    }
    dispatch_pending_metadata(state);
    if state.mirror.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_)) | Err(_))) { state.mirror = None; }
}

fn push_history(state: &mut State, item: QueueItem) {
    state.history.push_back(item);
    while state.history.len() > 50 { state.history.pop_front(); }
}

fn stop_current(state: &mut State) -> Result<(), String> {
    let mut close_error = None;
    let mut librewolf_closed = false;
    #[cfg(any(windows, target_os = "linux"))]
    if state.current.as_ref().is_some_and(|item| !item.local_playback && item.browser.as_deref().is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf"))) {
        if state.librewolf_bidi.is_none() {
            match librewolf_bidi_session() {
                Ok(session) => state.librewolf_bidi = Some(session),
                Err(error) => close_error = Some(error),
            }
        }
        if let Some(bidi) = state.librewolf_bidi.as_mut() {
            match bidi.close_browser() {
                Ok(()) => librewolf_closed = true,
                Err(error) => close_error = Some(error),
            }
        } else {
            eprintln!("Could not connect to the managed LibreWolf session to close the browser");
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    if state.current.is_none() {
        if let Some(bidi) = state.librewolf_bidi.as_mut() {
            match bidi.close_browser() {
                Ok(()) => librewolf_closed = true,
                Err(error) => close_error = Some(error),
            }
        } else if let Ok(mut bidi) = librewolf_bidi_session() {
            match bidi.close_browser() {
                Ok(()) => librewolf_closed = true,
                Err(error) => close_error = Some(error),
            }
        }
    }
    if librewolf_closed { state.librewolf_bidi = None; }
    if let Some(item) = state.current.as_ref().filter(|item| !item.local_playback && !item.browser.as_deref().is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf"))) {
        if let Some(player) = browser_player(item.browser.as_deref()) {
            if let Err(error) = run_playerctl(&player, &["quit"]) { eprintln!("Browser MPRIS quit request failed: {error}"); }
        }
    }
    if let Some(mut child) = state.player.take() { terminate_child(&mut child); }
    if let Some(socket) = state.mpv_socket.take() { let _ = std::fs::remove_file(socket); }
    state.current = None;
    close_error.map_or(Ok(()), Err)
}

fn respond(request: Request, status: u16, body: String, content_type: &str) {
    let header = Header::from_bytes("Content-Type", content_type).unwrap();
    let mut response = Response::from_string(body).with_status_code(StatusCode(status)).with_header(header);
    if let Some(origin) = request.headers().iter().find(|h| h.field.equiv("Origin")).map(|h| h.value.as_str()) {
        if remote_origin_allowed(origin) {
            for (name, value) in [("Access-Control-Allow-Origin", origin), ("Vary", "Origin")] {
                if let Ok(header) = Header::from_bytes(name, value) { response = response.with_header(header); }
            }
            if let Some(requested) = request.headers().iter().find(|h| h.field.equiv("Access-Control-Request-Private-Network"))
                .map(|h| h.value.as_str()).filter(|v| v.eq_ignore_ascii_case("true")) {
                let _ = requested;
                if let Ok(header) = Header::from_bytes("Access-Control-Allow-Private-Network", "true") { response = response.with_header(header); }
            }
            if let Ok(header) = Header::from_bytes("Access-Control-Allow-Methods", "GET, POST, OPTIONS") { response = response.with_header(header); }
            if let Ok(header) = Header::from_bytes("Access-Control-Allow-Headers", "Content-Type") { response = response.with_header(header); }
            if let Ok(header) = Header::from_bytes("Access-Control-Max-Age", "600") { response = response.with_header(header); }
        }
    }
    let _ = request.respond(response);
}

fn json<T: Serialize>(value: &T) -> String { serde_json::to_string(value).unwrap_or_else(|_| "{}".into()) }

fn run_playerctl(player: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("playerctl").arg("--player").arg(player).args(args).output()
        .map_err(|e| format!("could not run playerctl: {e}"))?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_string()); }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(windows)]
fn open_url_default(url: &str) -> Result<Child, String> {
    Command::new("explorer.exe").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("could not open URL with the Windows default browser: {e}"))
}

#[cfg(windows)]
fn open_dashboard_in_librewolf(url: &str) -> Result<Child, String> {
    let install_dir = env::current_exe().map_err(|error| format!("could not locate MediaBridge's install directory: {error}"))?
        .parent().ok_or("could not locate MediaBridge's install directory")?.to_path_buf();
    let browser = install_dir.join("librewolf").join("librewolf.exe");
    if !browser.is_file() { return Err(format!("bundled LibreWolf was not found at {}", browser.display())); }
    let profile = managed_librewolf_profile()
        .ok_or("could not locate the MediaBridge LibreWolf profile")?;
    ensure_librewolf_profile(&profile)?;
    Command::new(browser).arg("--profile").arg(profile).arg("--remote-debugging-port").arg("9222").arg(url)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|error| format!("could not open the dashboard in bundled LibreWolf: {error}"))
}

#[cfg(not(windows))]
fn open_url_default(url: &str) -> Result<Child, String> {
    Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|e| format!("could not open URL with xdg-open: {e}"))
}

#[cfg(target_os = "linux")]
fn open_dashboard_in_librewolf(url: &str) -> Result<Child, String> {
    let home = env::var_os("HOME").map(PathBuf::from)
        .ok_or("could not locate the installed MediaBridge LibreWolf launcher")?;
    let launcher = env::var_os("MEDIABRIDGE_LIBREWOLF_LAUNCHER")
        .map(PathBuf::from).unwrap_or_else(|| home.join(".local/bin/mediabridge-librewolf"));
    if !launcher.is_file() {
        return Err(format!("MediaBridge LibreWolf launcher was not found at {}", launcher.display()));
    }
    Command::new(&launcher).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|error| format!("could not open dashboard in bundled LibreWolf: {error}"))
}

#[cfg(windows)]
fn open_librewolf_sign_in(url: &str) -> Result<Child, String> {
    let install_dir = env::current_exe().map_err(|error| format!("could not locate MediaBridge's install directory: {error}"))?
        .parent().ok_or("could not locate MediaBridge's install directory")?.to_path_buf();
    let browser = install_dir.join("librewolf").join("librewolf.exe");
    if !browser.is_file() { return Err(format!("bundled LibreWolf was not found at {}", browser.display())); }
    let profile = managed_librewolf_profile().ok_or("could not locate the persistent MediaBridge LibreWolf profile")?;
    ensure_librewolf_profile(&profile)?;
    Command::new(browser).arg("--profile").arg(profile).arg(url)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|error| format!("could not open sign-in in bundled LibreWolf: {error}"))
}

#[cfg(target_os = "linux")]
fn open_librewolf_sign_in(url: &str) -> Result<Child, String> {
    let home = env::var_os("HOME").map(PathBuf::from)
        .ok_or("could not locate the installed MediaBridge LibreWolf launcher")?;
    let launcher = env::var_os("MEDIABRIDGE_LIBREWOLF_LAUNCHER")
        .map(PathBuf::from).unwrap_or_else(|| home.join(".local/bin/mediabridge-librewolf"));
    if !launcher.is_file() {
        return Err(format!("MediaBridge LibreWolf launcher was not found at {}", launcher.display()));
    }
    Command::new(&launcher).env("MEDIABRIDGE_LIBREWOLF_NO_BIDI", "1").arg(url)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|error| format!("could not open sign-in in bundled LibreWolf: {error}"))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_librewolf_sign_in(_url: &str) -> Result<Child, String> {
    Err("managed LibreWolf sign-in is available on Windows and Linux".into())
}

#[cfg(unix)]
fn mpv_socket_path() -> PathBuf {
    let base = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(env::temp_dir);
    base.join(format!("mediabridge-mpv-{}.sock", std::process::id()))
}

#[cfg(windows)]
fn mpv_socket_path() -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\mediabridge-mpv-{}", std::process::id()))
}

#[cfg(unix)]
fn mpv_ipc(socket_path: &Path, command: serde_json::Value) -> Result<serde_json::Value, String> {
    let stream = UnixStream::connect(socket_path)
        .map_err(|e| format!("could not connect to mpv IPC at {}: {e}", socket_path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    mpv_ipc_exchange(stream, command)
}

#[cfg(windows)]
fn mpv_ipc(socket_path: &Path, command: serde_json::Value) -> Result<serde_json::Value, String> {
    let stream = std::fs::OpenOptions::new().read(true).write(true).open(socket_path)
        .map_err(|e| format!("could not connect to mpv IPC at {}: {e}", socket_path.display()))?;
    mpv_ipc_exchange(stream, command)
}

fn mpv_ipc_exchange(stream: impl Read + Write, command: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut stream = stream;
    let payload = serde_json::json!({"command": command, "request_id": 1}).to_string();
    stream.write_all(format!("{payload}\n").as_bytes()).map_err(|e| format!("could not send mpv command: {e}"))?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).map_err(|e| format!("could not read mpv response: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|e| format!("invalid mpv IPC response: {e}"))?;
    if value.get("error").and_then(|v| v.as_str()).is_some_and(|e| e != "success") {
        return Err(format!("mpv rejected command: {}", value["error"].as_str().unwrap_or("unknown error")));
    }
    Ok(value)
}

#[cfg(not(any(unix, windows)))]
fn mpv_socket_path() -> PathBuf { PathBuf::new() }

#[cfg(not(any(unix, windows)))]
fn mpv_ipc(_socket_path: &Path, _command: serde_json::Value) -> Result<serde_json::Value, String> {
    Err("local mpv playback is not supported on this platform".into())
}

fn mpv_property(path: &Path, property: &str) -> Option<serde_json::Value> {
    mpv_ipc(path, serde_json::json!(["get_property", property])).ok()?.get("data").cloned()
}

fn queue_status(state: &State) -> Vec<QueueItemStatus> {
    state.queue.iter().map(|item| QueueItemStatus {
        url: safe_media_url(&item.url), title: item.title.clone(), uploader: item.uploader.clone(),
        thumbnail_url: item.thumbnail_url.clone(), channel_avatar_url: item.channel_avatar_url.clone(),
        channel_banner_url: item.channel_banner_url.clone(),
        local: item.local_playback, quality: item.quality.clone(), screen: item.screen,
        autoplay_generated: item.autoplay_generated,
    }).collect()
}

fn request_host_sleep() -> Result<(), String> {
    #[cfg(windows)]
    {
        thread::Builder::new().name("mediabridge-host-sleep".into()).spawn(|| {
            // Let the HTTP 202 response reach the caller before suspending the host.
            thread::sleep(Duration::from_secs(1));
            #[link(name = "PowrProf")]
            unsafe extern "system" {
                fn SetSuspendState(hibernate: u8, force: u8, disable_wake_events: u8) -> u8;
            }
            // Keep wake events enabled so a configured network adapter can wake the PC.
            if unsafe { SetSuspendState(0, 0, 0) } == 0 {
                eprintln!("Windows rejected the sleep request; check the MediaBridge process shutdown privilege and power policy");
            }
        }).map(|_| ()).map_err(|error| format!("could not schedule host sleep: {error}"))
    }
    #[cfg(target_os = "linux")]
    {
        thread::Builder::new().name("mediabridge-host-sleep".into()).spawn(|| {
            // Let the HTTP 202 response reach the caller before suspending the host.
            thread::sleep(Duration::from_secs(1));
            match Command::new("systemctl").arg("suspend").output() {
                Ok(output) if output.status.success() => {}
                Ok(output) => eprintln!("Linux rejected the sleep request: {}", String::from_utf8_lossy(&output.stderr).trim()),
                Err(error) => eprintln!("could not run systemctl suspend: {error}"),
            }
        }).map(|_| ()).map_err(|error| format!("could not schedule host sleep: {error}"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("host sleep is supported only on Windows and Linux".into())
    }
}

fn control_mpv(state: &mut State, operation: &str, value: Option<f64>) -> Result<(), String> {
    let path = state.mpv_socket.as_deref().ok_or("mpv IPC is not active")?;
    let command = match (operation, value) {
        ("pause", _) => serde_json::json!(["set_property", "pause", true]),
        ("resume", _) => serde_json::json!(["set_property", "pause", false]),
        ("play_pause", _) => serde_json::json!(["cycle", "pause"]),
        ("rewind", _) => serde_json::json!(["seek", -5, "relative"]),
        ("forward", _) => serde_json::json!(["seek", 5, "relative"]),
        ("seek", Some(position)) => serde_json::json!(["seek", position, "absolute"]),
        ("volume", Some(volume)) => serde_json::json!(["set_property", "volume", volume * 100.0]),
        ("volume_up", _) => serde_json::json!(["add", "volume", 5]),
        ("volume_down", _) => serde_json::json!(["add", "volume", -5]),
        _ => return Err(format!("invalid mpv control operation '{operation}'")),
    };
    mpv_ipc(path, command)?;
    if operation.starts_with("volume") {
        if let Some(volume) = mpv_property(path, "volume").and_then(|value| value.as_f64()) {
            let volume = volume.clamp(0.0, 100.0);
            state.last_mpv_volume = Some(volume / 100.0);
            // Keep volume feedback visible briefly after remote or dashboard changes.
            let _ = mpv_ipc(path, serde_json::json!(["show-text", format!("Volume: {}%", volume.round() as u8), 1500]));
        }
    }
    Ok(())
}

fn control_media(state: &mut State, operation: &str, value: Option<f64>) -> Result<(), String> {
    if state.current.as_ref().is_some_and(|item| item.local_playback) {
        return control_mpv(state, operation, value);
    }
    #[cfg(windows)]
    {
        if state.current.as_ref().is_none_or(|item| item.local_playback) {
            return Err("no active LibreWolf browser playback to control".into());
        }
        if state.current.as_ref().and_then(|item| item.browser.as_deref())
            .is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf")) {
            return control_librewolf(state, operation, value);
        }
        let twitch = state.current.as_ref().is_some_and(|item| twitch_url(&item.url));
        let key = match operation {
            "pause" | "resume" | "play_pause" => if twitch { 0x20 } else { 0x4B }, // Twitch uses Space; YouTube uses K.
            "rewind" => 0x25, // Left arrow: five seconds in the YouTube player.
            "forward" => 0x27, // Right arrow: five seconds in the YouTube player.
            "volume_up" => 0x26,
            "volume_down" => 0x28,
            "seek" | "volume" => return Err("absolute seek and volume controls are unavailable in browser playback on Windows; use Direct player for those controls".into()),
            _ => return Err(format!("invalid browser control operation '{operation}'")),
        };
        let _ = value;
        return send_windows_browser_key(key);
    }
    #[cfg(target_os = "linux")]
    if state.current.as_ref().and_then(|item| item.browser.as_deref())
        .is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf")) {
        return control_librewolf(state, operation, value);
    }
    #[cfg(not(windows))]
    {
    let player = controlled_player(state)?;
    let result = match operation {
        "pause" => run_playerctl(&player, &["pause"]),
        "resume" => run_playerctl(&player, &["play"]),
        "play_pause" => run_playerctl(&player, &["play-pause"]),
        "rewind" => run_playerctl(&player, &["position", "5-"]),
        "forward" => run_playerctl(&player, &["position", "5+"]),
        "volume_up" => run_playerctl(&player, &["volume", "0.05+"]),
        "volume_down" => run_playerctl(&player, &["volume", "0.05-"]),
        "seek" => { let position = value.ok_or("seek position is missing")?.to_string(); run_playerctl(&player, &["position", &position]) },
        "volume" => { let volume = value.ok_or("volume is missing")?.to_string(); run_playerctl(&player, &["volume", &volume]) },
        _ => return Err(format!("invalid media control operation '{operation}'")),
    };
    result.map(|_| ())
    }
}

#[cfg(windows)]
fn send_windows_browser_key(key: u16) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP};
    let input = |flags| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT {
        wVk: key, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0,
    } } };
    let inputs = [input(0), input(KEYEVENTF_KEYUP)];
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(format!("Windows did not send the browser media control: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn run_mouse(request: MouseRequest) -> Result<(), String> {
    let absolute = request.action == "move" || request.action == "click";
    let relative = request.action == "move_relative";
    let click_current = request.action == "click_current";
    if absolute && (!(0..=32767).contains(&request.x) || !(0..=32767).contains(&request.y)) {
        return Err("mouse x and y must be between 0 and 32767".into());
    }
    if relative && (!( -32767..=32767).contains(&request.dx) || !( -32767..=32767).contains(&request.dy)) {
        return Err("mouse dx and dy must be between -32767 and 32767".into());
    }
    if !absolute && !relative && !click_current {
        return Err("mouse action must be 'move', 'click', 'move_relative', or 'click_current'".into());
    }
    #[cfg(windows)]
    { return run_mouse_windows(request, absolute, relative, click_current); }
    #[cfg(not(windows))]
    {
    let button = match request.button.as_str() {
        "left" => ("1", "0xC0"),
        "right" => ("3", "0xC1"),
        "middle" => ("2", "0xC2"),
        _ => return Err("mouse button must be 'left', 'right', or 'middle'".into()),
    };
    let wayland = wayland_session();
    println!("Sending mouse input through {}", if wayland { "ydotool (Wayland)" } else { "xdotool (X11)" });
    let (program, args): (&str, Vec<String>) = if wayland {
        let mut args = if absolute { vec!["mousemove".into(), "--absolute".into(), "-x".into(), request.x.to_string(), "-y".into(), request.y.to_string()] }
            else if relative { vec!["mousemove".into(), "-x".into(), request.dx.to_string(), "-y".into(), request.dy.to_string()] }
            else { Vec::new() };
        if request.action == "click" || click_current { if !args.is_empty() { args.push("&&".into()); } args.extend(["click".into(), button.1.into()]); }
        ("ydotool", args)
    } else {
        let mut args = if absolute { vec!["mousemove".into(), "--sync".into(), request.x.to_string(), request.y.to_string()] }
            else if relative { vec!["mousemove_relative".into(), "--sync".into(), request.dx.to_string(), request.dy.to_string()] }
            else { Vec::new() };
        if request.action == "click" || click_current { if !args.is_empty() { args.push("&&".into()); } args.extend(["click".into(), button.0.into()]); }
        ("xdotool", args)
    };
    // Run each input operation separately; the `&&` marker above is only an internal separator.
    let mut operations = args.split(|arg| arg == "&&");
    for operation in operations.by_ref() {
        let mut command = Command::new(program);
        command.args(operation);
        if wayland && Path::new("/run/ydotool/socket").exists() {
            // Prefer the installer-managed shared service socket over stale
            // per-session YDOTOOL_SOCKET values from a desktop environment.
            command.env("YDOTOOL_SOCKET", "/run/ydotool/socket");
        }
        let output = command.output().map_err(|e| {
            if wayland { format!("could not run ydotool: {e}; on Wayland, install ydotool and start ydotoold with access to /dev/uinput") }
            else { format!("could not run xdotool: {e}; install xdotool or run MediaBridge inside the X11 desktop session") }
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            let detail = if detail.is_empty() { format!("process exited with {}", output.status) } else { detail };
            return Err(if wayland { format!("ydotool failed: {detail}; check the ydotoold socket and /dev/uinput permissions") } else { format!("xdotool failed: {detail}") });
        }
    }
    Ok(())
    }
}

#[cfg(windows)]
fn run_mouse_windows(request: MouseRequest, absolute: bool, relative: bool, click_current: bool) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT, MOUSEEVENTF_ABSOLUTE,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP,
    };
    let mut events = Vec::new();
    if absolute || relative {
        let (dx, dy, flags) = if absolute {
            (request.x * 65535 / 32767, request.y * 65535 / 32767, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)
        } else { (request.dx, request.dy, MOUSEEVENTF_MOVE) };
        events.push((dx, dy, 0, flags));
    }
    if request.action == "click" || click_current {
        let (down, up) = match request.button.as_str() {
            "left" => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            _ => return Err("mouse button must be 'left', 'right', or 'middle'".into()),
        };
        events.push((0, 0, 0, down));
        events.push((0, 0, 0, up));
    }
    let inputs: Vec<INPUT> = events.into_iter().map(|(dx, dy, mouse_data, flags)| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: mouse_data, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }).collect();
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(format!("Windows did not accept the requested mouse input: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn send_desktop_shortcut(shortcut: &str) -> Result<(), String> {
    #[cfg(windows)]
    { return send_desktop_shortcut_windows(shortcut); }
    #[cfg(not(windows))]
    {
    let wayland = wayland_session();
    let (program, args): (&str, Vec<&str>) = match (wayland, shortcut) {
        (true, "youtube-f") => ("ydotool", vec!["key", "33:1", "33:0"]),
        (true, "twitch-alt-t") => ("ydotool", vec!["key", "56:1", "20:1", "20:0", "56:0"]),
        (false, "youtube-f") => ("xdotool", vec!["key", "f"]),
        (false, "twitch-alt-t") => ("xdotool", vec!["key", "alt+t"]),
        _ => return Err(format!("unknown desktop shortcut '{shortcut}'")),
    };
    let mut command = Command::new(program);
    command.args(args);
    if wayland && Path::new("/run/ydotool/socket").exists() {
        command.env("YDOTOOL_SOCKET", "/run/ydotool/socket");
    }
    let output = command.output().map_err(|e| format!("could not run {program} for {shortcut}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!("{program} could not send {shortcut}: {}", if detail.is_empty() { output.status.to_string() } else { detail }));
    }
    Ok(())
    }
}

#[cfg(windows)]
fn send_desktop_shortcut_windows(shortcut: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_MENU,
    };
    let key_input = |key: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: key, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = match shortcut {
        "youtube-f" => vec![key_input(b'F' as u16, 0), key_input(b'F' as u16, KEYEVENTF_KEYUP)],
        "twitch-alt-t" => vec![
            key_input(VK_MENU as u16, 0), key_input(b'T' as u16, 0),
            key_input(b'T' as u16, KEYEVENTF_KEYUP), key_input(VK_MENU as u16, KEYEVENTF_KEYUP),
        ],
        _ => return Err(format!("unknown desktop shortcut '{shortcut}'")),
    };
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(format!("Windows did not accept the requested keyboard input: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn browser_player(browser_arg: Option<&str>) -> Option<String> {
    let output = Command::new("playerctl").arg("--list-all").output().ok()?;
    if !output.status.success() { return None; }
    let players = String::from_utf8_lossy(&output.stdout).lines()
        .map(str::trim)
        .map(str::to_string).collect::<Vec<_>>();
    let prefixes: Vec<String> = if let Some(browser) = browser_arg {
        let name = Path::new(browser).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_else(|| browser.to_lowercase());
        let canonical = match name.as_str() { "chromium-browser" => "chromium", "google-chrome-stable" => "google-chrome", "mediabridge-librewolf" => "firefox", _ => name.as_str() };
        vec![canonical.to_string()]
    } else { ["chromium", "google-chrome", "firefox", "brave", "vivaldi", "microsoft-edge", "min"].iter().map(|s| s.to_string()).collect() };
    players.into_iter().find(|player| {
        let name = player.to_lowercase();
        prefixes.iter().any(|prefix| name == *prefix || name.starts_with(&format!("{prefix}.")))
    })
}

fn media_status(state: &mut State) -> Status {
    #[cfg(any(windows, target_os = "linux"))]
    let bidi_video = refresh_librewolf_video(state);
    if state.current.as_ref().is_some_and(|item| item.local_playback) {
        if let Some(socket) = state.mpv_socket.as_deref() {
            let title = mpv_property(socket, "media-title").and_then(|v| v.as_str().map(str::to_string));
            let position_seconds = mpv_property(socket, "time-pos").and_then(|v| v.as_f64());
            let duration_seconds = mpv_property(socket, "duration").and_then(|v| v.as_f64());
            let volume = mpv_property(socket, "volume").and_then(|v| v.as_f64()).map(|v| (v / 100.0).clamp(0.0, 1.0));
            if let Some(volume) = volume { state.last_mpv_volume = Some(volume); }
            let paused = mpv_property(socket, "pause").and_then(|v| v.as_bool());
            let playback_state = match paused { Some(true) => "paused", Some(false) => "playing", None => "unknown" }.to_string();
            return Status {
                current: state.current.as_ref().map(|item| safe_media_url(&item.url)),
                queued: state.queue.iter().map(|item| safe_media_url(&item.url)).collect(),
                queue_items: queue_status(state),
                playing: playback_state == "playing", mirroring: state.mirror.is_some(),
                playback_state, playback_error: state.playback_error.clone(), title, artist: None, album: None,
                artwork_url: state.current.as_ref().and_then(|item| item.thumbnail_url.clone()),
                channel_avatar_url: state.current.as_ref().and_then(|item| item.channel_avatar_url.clone()),
                channel_banner_url: state.current.as_ref().and_then(|item| item.channel_banner_url.clone()),
                media_url: state.current.as_ref().map(|item| safe_media_url(&item.url)), position_seconds,
                duration_seconds, volume, can_seek: duration_seconds.is_some_and(|d| d > 0.0),
                previous_available: !state.history.is_empty(),
                autoplay_enabled: state.autoplay_enabled,
                autoplay_until_epoch_ms: autoplay_expiration_epoch_ms(state),
            };
        }
    }
    let browser = state.current.as_ref().and_then(|item| item.browser.as_deref());
    let media = browser_player(browser).and_then(|p| {
        let formatted = run_playerctl(&p, &["metadata", "--format", "{{title}}\t{{artist}}\t{{album}}\t{{mpris:artUrl}}\t{{mpris:length}}\t{{xesam:url}}\t{{position}}\t{{volume}}\t{{status}}"]).ok()?;
        let fields = formatted.split('\t').collect::<Vec<_>>();
        if fields.len() < 9 { return None; }
        let opt_text = |s: &str| if s.is_empty() { None } else { Some(s.to_string()) };
        let seconds = |s: &str| s.parse::<f64>().ok().map(|us| us / 1_000_000.0);
        Some((opt_text(fields[0]), opt_text(fields[1]), opt_text(fields[2]), opt_text(fields[3]),
              opt_text(fields[5]), seconds(fields[4]), seconds(fields[6]), fields[7].parse::<f64>().ok(), fields[8].to_lowercase()))
    });
    let (mut title, mut artist, album, artwork_url, media_url, mut duration_seconds, mut position_seconds, mut volume, mut mpris_state) =
        media.unwrap_or((None, None, None, None, None, None, None, None, String::new()));
    #[cfg(any(windows, target_os = "linux"))]
    if let Some(video) = bidi_video {
        title = video.title.or(title);
        duration_seconds = video.duration.or(duration_seconds);
        position_seconds = video.current_time.or(position_seconds);
        volume = video.volume.or(volume);
        mpris_state = match video.paused { Some(true) => "paused", Some(false) => "playing", None => "unknown" }.into();
    }
    if let Some(current) = state.current.as_ref() {
        if title.is_none() { title = current.title.clone(); }
        if artist.is_none() { artist = current.uploader.clone(); }
    }
    let artwork_url = artwork_url.or_else(|| state.current.as_ref().and_then(|item| item.thumbnail_url.clone()));
    let playback_state = if !mpris_state.is_empty() { mpris_state }
        else if state.mirror.is_some() { "mirroring".into() }
        else if cfg!(windows) && state.current.as_ref().is_some_and(|item| !item.local_playback) { "browser session (state unavailable)".into() }
        else if state.current.is_some() { "unknown".into() }
        else { "idle".into() };
    Status {
        current: state.current.as_ref().map(|item| safe_media_url(&item.url)), queued: state.queue.iter().map(|item| safe_media_url(&item.url)).collect(),
        queue_items: queue_status(state),
        playing: playback_state == "playing", mirroring: state.mirror.is_some(),
        playback_state, playback_error: state.playback_error.clone(), title, artist, album, artwork_url, media_url: media_url.map(|url| safe_media_url(&url)), position_seconds,
        channel_avatar_url: state.current.as_ref().and_then(|item| item.channel_avatar_url.clone()),
        channel_banner_url: state.current.as_ref().and_then(|item| item.channel_banner_url.clone()),
        duration_seconds, volume, can_seek: duration_seconds.is_some_and(|d| d > 0.0),
        previous_available: !state.history.is_empty(),
        autoplay_enabled: state.autoplay_enabled,
        autoplay_until_epoch_ms: autoplay_expiration_epoch_ms(state),
    }
}

fn autoplay_expiration_epoch_ms(state: &State) -> Option<u64> {
    let remaining = state.autoplay_until?.checked_duration_since(std::time::Instant::now())?;
    Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64 + remaining.as_millis() as u64)
}

#[cfg(not(windows))]
fn controlled_player(state: &State) -> Result<String, String> {
    let browser = state.current.as_ref().and_then(|item| item.browser.as_deref());
    browser_player(browser).ok_or_else(|| "no browser MPRIS player found; start playback in the active desktop session".into())
}

fn handle(mut request: Request, cfg: &Config, state: &mut State) {
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    if let Some(origin) = request.headers().iter().find(|h| h.field.equiv("Origin")).map(|h| h.value.as_str()) {
        let local_origin = request.headers().iter().find(|h| h.field.equiv("Host"))
            .map(|h| format!("http://{}", h.value.as_str()));
        let allowed_remote = remote_origin_allowed(origin);
        let local_dashboard = local_origin.as_deref() == Some(origin);
        if (!allowed_remote && !local_dashboard) || (path.starts_with("/settings/") && !local_dashboard) {
            respond(request, 403, "{\"error\":\"origin is not allowed\"}".into(), "application/json");
            return;
        }
    }
    if method == Method::Options {
        respond(request, 204, String::new(), "text/plain; charset=utf-8");
        return;
    }
    if !(method == Method::Get && path == "/status") {
        println!("Received {} {}", method, path);
    }
    if method == Method::Get && (path == "/" || path == "/index.html") {
        respond(request, 200, include_str!("web/index.html").to_string(), "text/html; charset=utf-8"); return;
    }
    if method == Method::Get && path == "/preferences" {
        respond(request, 200, json(&current_playback_preferences(cfg)), "application/json"); return;
    }
    if method == Method::Get && path == "/platform" {
        respond(request, 200, json(&serde_json::json!({"os": env::consts::OS})), "application/json"); return;
    }
    if method == Method::Post && path == "/settings/cookies/check" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: CookieCheckRequest = match serde_json::from_str(&body) { Ok(value) => value, Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; } };
        if !matches!(parsed.cookies_browser.as_str(), "auto" | "librewolf" | "chrome" | "chromium" | "edge" | "firefox" | "brave" | "opera" | "vivaldi" | "whale") {
            respond(request, 400, "{\"error\":\"choose a supported browser to check cookie access\"}".into(), "application/json"); return;
        }
        if !matches!(parsed.site.as_str(), "youtube" | "twitch") {
            respond(request, 400, "{\"error\":\"choose YouTube or Twitch to check account cookies\"}".into(), "application/json"); return;
        }
        if parsed.cookies_profile.as_deref().is_some_and(|profile| profile.contains(',') || profile.contains('\n') || profile.contains('\r')) {
            respond(request, 400, "{\"error\":\"browser profile must not contain commas or line breaks\"}".into(), "application/json"); return;
        }
        match check_cookie_access(&parsed.cookies_browser, parsed.cookies_profile.as_deref(), &parsed.site) {
            Ok(result) => respond(request, 200, json(&result), "application/json"),
            Err(error) => respond(request, 422, json(&serde_json::json!({"error": error})), "application/json"),
        }
        return;
    }
    if method == Method::Post && path == "/settings/accounts/login" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return;
        }
        let parsed: serde_json::Value = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; }
        };
        let site = parsed.get("site").and_then(|value| value.as_str()).unwrap_or("");
        let (url, site_label) = match site {
            "youtube" => ("https://www.youtube.com/", "YouTube"),
            "twitch" => ("https://www.twitch.tv/login", "Twitch"),
            _ => { respond(request, 400, "{\"error\":\"choose YouTube or Twitch to sign in\"}".into(), "application/json"); return; }
        };
        let browser_needs_close = state.current.as_ref().is_some_and(|item| item.local_playback
            || !item.browser.as_deref().is_some_and(|browser| browser.to_ascii_lowercase().contains("librewolf")));
        if let Err(error) = stop_current(state) {
            respond(request, 409, json(&serde_json::json!({"error": format!("could not restart LibreWolf for sign-in: {error}")})), "application/json");
            return;
        }
        // stop_current closes LibreWolf when it owns the current playback. If
        // mpv or another browser was active, close the dashboard's LibreWolf too.
        #[cfg(any(windows, target_os = "linux"))]
        if browser_needs_close {
            match librewolf_bidi_session().and_then(|mut bidi| bidi.close_browser()) {
                Ok(()) => state.librewolf_bidi = None,
                Err(error) => {
                    respond(request, 409, json(&serde_json::json!({"error": format!("could not close the managed LibreWolf dashboard before sign-in: {error}")})), "application/json");
                    return;
                }
            }
        }
        match open_librewolf_sign_in(url) {
            Ok(_child) => respond(request, 202, json(&serde_json::json!({"ok": true, "site": site_label,
                "message": format!("Sign in to {site_label}, close LibreWolf when finished, then reopen the MediaBridge dashboard.")})), "application/json"),
            Err(error) => respond(request, 500, json(&serde_json::json!({"error": error})), "application/json"),
        }
        return;
    }
    if method == Method::Post && path == "/settings/preferences" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return;
        }
        let parsed: PlaybackPreferencesRequest = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; }
        };
        let preferences = PlaybackPreferences::from(parsed);
        match save_playback_preferences(preferences.clone()) {
            Ok(()) => respond(request, 200, json(&preferences), "application/json"),
            Err(error) => {
                let status = if error.starts_with("could not save") || error.starts_with("could not read") { 500 } else { 400 };
                respond(request, status, json(&serde_json::json!({"error": error})), "application/json");
            }
        }
        return;
    }
    if method == Method::Get && path == "/settings/port" {
        let port = cfg.bind.parse::<SocketAddr>().map(|address| address.port()).unwrap_or(8765);
        respond(request, 200, json(&serde_json::json!({"port": port})), "application/json"); return;
    }
    if method == Method::Post && path == "/settings/port" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return;
        }
        let parsed: PortChangeRequest = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; }
        };
        if parsed.port == 0 {
            respond(request, 400, "{\"error\":\"port must be between 1 and 65535\"}".into(), "application/json"); return;
        }
        if state.restart_bind.is_some() {
            respond(request, 409, "{\"error\":\"a server restart is already pending\"}".into(), "application/json"); return;
        }
        let current = match cfg.bind.parse::<SocketAddr>() {
            Ok(address) => address,
            Err(error) => { respond(request, 500, json(&serde_json::json!({"error": format!("cannot change port for bind address: {error}")})), "application/json"); return; }
        };
        if parsed.port == current.port() {
            respond(request, 200, json(&serde_json::json!({"port": parsed.port, "unchanged": true})), "application/json"); return;
        }
        let new_bind = SocketAddr::new(current.ip(), parsed.port).to_string();
        let availability = match TcpListener::bind(&new_bind) {
            Ok(listener) => { drop(listener); Ok(()) }
            Err(error) => Err(error),
        };
        if let Err(error) = availability {
            respond(request, 409, json(&serde_json::json!({"error": format!("port {} is not available: {error}", parsed.port)})), "application/json"); return;
        }
        if let Err(error) = save_bind_address(&new_bind) {
            respond(request, 500, json(&serde_json::json!({"error": error})), "application/json"); return;
        }
        state.restart_bind = Some(new_bind);
        respond(request, 202, json(&serde_json::json!({"port": parsed.port, "restart": true})), "application/json");
        return;
    }
    if method == Method::Get && path == "/settings/remote-origins" {
        let origins = REMOTE_ORIGINS.get().and_then(|list| list.read().ok()).map(|list| list.clone()).unwrap_or_default();
        respond(request, 200, json(&serde_json::json!({"remote_origin_servers": origins})), "application/json"); return;
    }
    if method == Method::Get && path == "/settings/port" {
        let port = cfg.bind.parse::<SocketAddr>().map(|address| address.port()).unwrap_or(8765);
        respond(request, 200, json(&serde_json::json!({"port": port})), "application/json"); return;
    }
    if method == Method::Post && path == "/settings/port" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return;
        }
        let parsed: PortChangeRequest = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; }
        };
        if parsed.port == 0 {
            respond(request, 400, "{\"error\":\"port must be between 1 and 65535\"}".into(), "application/json"); return;
        }
        if state.restart_bind.is_some() {
            respond(request, 409, "{\"error\":\"a server restart is already pending\"}".into(), "application/json"); return;
        }
        let current = match cfg.bind.parse::<SocketAddr>() {
            Ok(address) => address,
            Err(error) => { respond(request, 500, json(&serde_json::json!({"error": format!("cannot change port for bind address: {error}")})), "application/json"); return; }
        };
        if parsed.port == current.port() {
            respond(request, 200, json(&serde_json::json!({"port": parsed.port, "unchanged": true})), "application/json"); return;
        }
        let new_bind = SocketAddr::new(current.ip(), parsed.port).to_string();
        let availability = match TcpListener::bind(&new_bind) {
            Ok(listener) => { drop(listener); Ok(()) }
            Err(error) => Err(error),
        };
        if let Err(error) = availability {
            respond(request, 409, json(&serde_json::json!({"error": format!("port {} is not available: {error}", parsed.port)})), "application/json"); return;
        }
        if let Err(error) = save_bind_address(&new_bind) {
            respond(request, 500, json(&serde_json::json!({"error": error})), "application/json"); return;
        }
        state.restart_bind = Some(new_bind);
        respond(request, 202, json(&serde_json::json!({"port": parsed.port, "restart": true})), "application/json");
        return;
    }
    if method == Method::Post && path == "/settings/remote-origins" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() {
            respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return;
        }
        let parsed: RemoteOriginsRequest = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; }
        };
        match save_remote_origins(parsed.remote_origin_servers) {
            Ok(origins) => respond(request, 200, json(&serde_json::json!({"remote_origin_servers": origins})), "application/json"),
            Err(error) => {
                let status = if error.starts_with("invalid web origin") { 400 } else { 500 };
                respond(request, status, json(&serde_json::json!({"error": error})), "application/json");
            }
        }
        return;
    }
    if method == Method::Post && path == "/resolve" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: LinkRequest = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        let url = match clean_media_url(&parsed.url, false) { Ok(url) => url, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e})), "application/json"); return; } };
        match resolve_media_url(&url) {
            Ok(metadata) => respond(request, 200, json(&metadata), "application/json"),
            Err(e) => respond(request, 422, json(&serde_json::json!({"error": e, "url": url})), "application/json"),
        }
        return;
    }
    if method == Method::Post && path == "/resolve_async" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: LinkRequest = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        let url = match clean_media_url(&parsed.url, false) { Ok(url) => url, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e})), "application/json"); return; } };
        if state.metadata_tx.is_some() {
            if !state.pending_metadata.iter().any(|pending| pending == &url) {
                state.pending_metadata.push_back(url);
            }
            respond(request, 202, "{\"ok\":true}".into(), "application/json");
        } else {
            respond(request, 503, "{\"error\":\"metadata resolver unavailable\"}".into(), "application/json");
        }
        return;
    }
    if method == Method::Get && path == "/status" {
        let value = media_status(state);
        respond(request, 200, json(&value), "application/json"); return;
    }
    if method == Method::Post && path == "/autoplay" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: AutoplayRequest = match serde_json::from_str(&body) { Ok(value) => value, Err(error) => { respond(request, 400, json(&serde_json::json!({"error": error.to_string()})), "application/json"); return; } };
        if parsed.duration_minutes.is_some_and(|minutes| !(1..=1440).contains(&minutes)) { respond(request, 400, "{\"error\":\"autoplay duration must be from 1 to 1440 minutes\"}".into(), "application/json"); return; }
        set_autoplay(state, parsed.enabled, parsed.duration_minutes);
        respond(request, 200, "{\"ok\":true}".into(), "application/json"); return;
    }
    if method == Method::Post && ["/play_playlist", "/queue_playlist", "/queue_playlist_next"].contains(&path.as_str()) {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let mut parsed: QueueItem = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        if parsed.autoplay_minutes.is_some_and(|minutes| !(1..=1440).contains(&minutes)) { respond(request, 400, "{\"error\":\"autoplay duration must be from 1 to 1440 minutes\"}".into(), "application/json"); return; }
        parsed.url = match clean_media_url(&parsed.url, false) { Ok(url) => url, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e})), "application/json"); return; } };
        if !youtube_url(&parsed.url) || !parsed.url.split_once('?').is_some_and(|(_, query)| query.split('&').any(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            key == "list" && !value.trim().is_empty()
        })) {
            respond(request, 400, "{\"error\":\"a YouTube playlist link with a list parameter is required\"}".into(), "application/json"); return;
        }
        if parsed.screen.or(current_playback_preferences(cfg).default_screen).is_some_and(|screen| screen > 32) {
            respond(request, 400, "{\"error\":\"display index must be from 0 to 32\"}".into(), "application/json"); return;
        }
        parsed.browser = parsed.browser.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        let urls = match expand_youtube_playlist(&parsed.url) {
            Ok(urls) => urls,
            Err(e) => { respond(request, 422, json(&serde_json::json!({"error": e})), "application/json"); return; }
        };
        if let Some(enabled) = parsed.autoplay { set_autoplay(state, enabled, parsed.autoplay_minutes); }
        let count = urls.len();
        let mut items = urls.into_iter().map(|url| { let mut item = parsed.clone(); item.url = url; item.title = None; item.uploader = None; item.thumbnail_url = None; item.channel_avatar_url = None; item.channel_banner_url = None; item }).collect::<Vec<_>>();
        if path == "/play_playlist" {
            state.playback_epoch = state.playback_epoch.wrapping_add(1);
            state.playback_error = None;
            if let Some(current) = state.current.clone() { push_history(state, current); }
            state.queue.clear();
            let _ = stop_current(state);
            let first = items.remove(0);
            if let Err(e) = start_url(state, cfg, first) { respond(request, 500, json(&serde_json::json!({"error": e})), "application/json"); return; }
            state.queue.extend(items);
        } else if path == "/queue_playlist_next" && state.current.is_some() {
            state.queue.retain(|item| !item.autoplay_generated);
            for item in items.drain(..).rev() { state.queue.push_front(item); }
        } else if state.current.is_some() {
            for item in items { enqueue_explicit_item(state, item); }
        } else {
            let first = items.remove(0);
            if let Err(e) = start_url(state, cfg, first) { respond(request, 500, json(&serde_json::json!({"error": e})), "application/json"); return; }
            state.queue.extend(items);
        }
        for item in &state.queue {
            if item.title.is_none() && !state.pending_queue_metadata.iter().any(|url| same_media_url(url, &item.url)) {
                state.pending_queue_metadata.push_back(item.url.clone());
            }
        }
        respond(request, 202, json(&serde_json::json!({"ok": true, "added": count})), "application/json"); return;
    }
    if method == Method::Post && ["/play", "/queue", "/queue_next"].contains(&path.as_str()) {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let mut parsed: QueueItem = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        if parsed.autoplay_minutes.is_some_and(|minutes| !(1..=1440).contains(&minutes)) { respond(request, 400, "{\"error\":\"autoplay duration must be from 1 to 1440 minutes\"}".into(), "application/json"); return; }
        if parsed.screen.or(current_playback_preferences(cfg).default_screen).is_some_and(|screen| screen > 32) {
            respond(request, 400, "{\"error\":\"display index must be from 0 to 32\"}".into(), "application/json"); return;
        }
        parsed.url = match clean_media_url(&parsed.url, parsed.local_playback) { Ok(url) => url, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e})), "application/json"); return; } };
        if parsed.url.starts_with("smb://") && !parsed.local_playback { respond(request, 400, "{\"error\":\"SMB links require local mpv playback\"}".into(), "application/json"); return; }
        if let Some(enabled) = parsed.autoplay { set_autoplay(state, enabled, parsed.autoplay_minutes); }
        parsed.browser = parsed.browser.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        if path == "/queue_next" && state.current.is_some() { state.queue.retain(|item| !item.autoplay_generated); state.queue.push_front(parsed); }
        else if path == "/queue" && state.current.is_some() { enqueue_explicit_item(state, parsed); }
        else {
            if path == "/play" {
                state.playback_epoch = state.playback_epoch.wrapping_add(1);
                state.playback_error = None;
                if let Some(current) = state.current.clone() { push_history(state, current); }
                state.queue.clear();
            }
            let _ = stop_current(state);
            if let Err(e) = start_url(state, cfg, parsed) { respond(request, 500, json(&serde_json::json!({"error": e})), "application/json"); return; }
        }
        respond(request, 202, "{\"ok\":true}".into(), "application/json"); return;
    }
    if method == Method::Post && ["/queue/remove", "/queue/move", "/queue/promote", "/queue/play"].contains(&path.as_str()) {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: QueueIndexRequest = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        if parsed.index >= state.queue.len() { respond(request, 404, "{\"error\":\"queue index not found\"}".into(), "application/json"); return; }
        if path == "/queue/play" {
            let item = state.queue.remove(parsed.index).expect("validated queue index");
            if let Some(current) = state.current.clone() { push_history(state, current); }
            state.playback_epoch = state.playback_epoch.wrapping_add(1);
            state.playback_error = None;
            let _ = stop_current(state);
            match start_url(state, cfg, item) {
                Ok(()) => respond(request, 202, "{\"ok\":true}".into(), "application/json"),
                Err(e) => respond(request, 500, json(&serde_json::json!({"error": e})), "application/json"),
            }
            return;
        } else if path == "/queue/remove" {
            state.queue.remove(parsed.index);
        } else if path == "/queue/promote" {
            if let Some(item) = state.queue.get_mut(parsed.index) { item.autoplay_generated = false; }
        } else {
            let Some(to) = parsed.to else { respond(request, 400, "{\"error\":\"target queue index is required\"}".into(), "application/json"); return; };
            if to >= state.queue.len() { respond(request, 400, "{\"error\":\"target queue index is out of range\"}".into(), "application/json"); return; }
            let item = state.queue.remove(parsed.index).expect("validated queue index");
            state.queue.insert(to, item);
        }
        respond(request, 200, "{\"ok\":true}".into(), "application/json"); return;
    }
    if method == Method::Post && path == "/close" {
        state.playback_epoch = state.playback_epoch.wrapping_add(1);
        state.playback_error = None;
        set_autoplay(state, false, None);
        if let Some(cancel) = state.metadata_cancel.take() { cancel.store(true, Ordering::Release); }
        state.metadata_in_flight = false;
        state.metadata_in_flight_url = None;
        state.autoplay_waiting_to_advance = false;
        state.pending_metadata.clear();
        state.pending_queue_metadata.clear();
        let close_result = stop_current(state);
        if let Some(mut child) = state.mirror.take() { let _ = child.kill(); let _ = child.wait(); }
        state.queue.clear();
        state.history.clear();
        match close_result {
            Ok(()) => respond(request, 200, "{\"ok\":true}".into(), "application/json"),
            Err(error) => respond(request, 409, json(&serde_json::json!({"error": format!("playback state was cleared but LibreWolf did not confirm it closed: {error}")})), "application/json"),
        }
        return;
    }
    if method == Method::Post && path == "/sleep" {
        match request_host_sleep() {
            Ok(()) => respond(request, 202, "{\"ok\":true}\n".into(), "application/json"),
            Err(error) if !cfg!(any(windows, target_os = "linux")) => respond(request, 501, json(&serde_json::json!({"error": error})), "application/json"),
            Err(error) => respond(request, 500, json(&serde_json::json!({"error": error})), "application/json"),
        }
        return;
    }
    if method == Method::Post && ["/pause", "/resume", "/play_pause", "/rewind", "/forward", "/volume_up", "/volume_down", "/zoom_in", "/zoom_out", "/zoom_reset"].contains(&path.as_str()) {
        let result = match path.as_str() {
            "/pause" => control_media(state, "pause", None),
            "/resume" => control_media(state, "resume", None),
            "/play_pause" => control_media(state, "play_pause", None),
            "/rewind" => control_media(state, "rewind", None),
            "/forward" => control_media(state, "forward", None),
            "/volume_up" => control_media(state, "volume_up", None),
            "/volume_down" => control_media(state, "volume_down", None),
            "/zoom_in" => control_media(state, "zoom_in", None),
            "/zoom_out" => control_media(state, "zoom_out", None),
            _ => control_media(state, "zoom_reset", None),
        };
        match result { Ok(_) => respond(request, 200, "{\"ok\":true}".into(), "application/json"), Err(e) => respond(request, 409, json(&serde_json::json!({"error": e})), "application/json") }
        return;
    }
    if method == Method::Post && path == "/seek" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: SeekRequest = match serde_json::from_str::<SeekRequest>(&body) { Ok(v) if v.position.is_finite() && v.position >= 0.0 => v, _ => { respond(request, 400, "{\"error\":\"position must be a non-negative number of seconds\"}".into(), "application/json"); return; } };
        match control_media(state, "seek", Some(parsed.position)) { Ok(_) => respond(request, 200, "{\"ok\":true}".into(), "application/json"), Err(e) => respond(request, 409, json(&serde_json::json!({"error": e})), "application/json") }
        return;
    }
    if method == Method::Post && path == "/volume" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: VolumeRequest = match serde_json::from_str::<VolumeRequest>(&body) { Ok(v) if v.volume.is_finite() && (0.0..=1.0).contains(&v.volume) => v, _ => { respond(request, 400, "{\"error\":\"volume must be between 0 and 1\"}".into(), "application/json"); return; } };
        match control_media(state, "volume", Some(parsed.volume)) { Ok(_) => respond(request, 200, "{\"ok\":true}".into(), "application/json"), Err(e) => respond(request, 409, json(&serde_json::json!({"error": e})), "application/json") }
        return;
    }
    if method == Method::Post && path == "/mouse" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: MouseRequest = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        match run_mouse(parsed) {
            Ok(()) => respond(request, 200, "{\"ok\":true}".into(), "application/json"),
            Err(e) => respond(request, 400, json(&serde_json::json!({"error": e})), "application/json"),
        }
        return;
    }
    if method == Method::Post && path == "/previous" {
        let previous = match state.history.pop_back() {
            Some(item) => item,
            None => { respond(request, 409, "{\"error\":\"no previous media\"}".into(), "application/json"); return; }
        };
        if let Some(current) = state.current.clone() { state.queue.push_front(current); }
        state.playback_epoch = state.playback_epoch.wrapping_add(1);
        state.playback_error = None;
        let _ = stop_current(state);
        match start_url(state, cfg, previous) {
            Ok(()) => respond(request, 202, "{\"ok\":true}".into(), "application/json"),
            Err(e) => respond(request, 500, json(&serde_json::json!({"error": e})), "application/json"),
        }
        return;
    }
    if method == Method::Post && path == "/next" {
        let next = match state.queue.pop_front() {
            Some(item) => item,
            None => { respond(request, 409, "{\"error\":\"queue is empty\"}".into(), "application/json"); return; }
        };
        if let Some(current) = state.current.clone() { push_history(state, current); }
        state.playback_epoch = state.playback_epoch.wrapping_add(1);
        state.playback_error = None;
        let _ = stop_current(state);
        match start_url(state, cfg, next) {
            Ok(()) => respond(request, 202, "{\"ok\":true}".into(), "application/json"),
            Err(e) => respond(request, 500, json(&serde_json::json!({"error": e})), "application/json"),
        }
        return;
    }
    if method == Method::Get && path == "/sunshine/pair/status" {
        let status = moonlight_pair_status(state);
        respond(request, 200, json(&status), "application/json");
        return;
    }
    if method == Method::Post && path == "/sunshine/pair" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: MirrorRequest = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        let host = parsed.host.trim();
        if host.is_empty() || host.len() > 253 || host.chars().any(char::is_control) {
            respond(request, 400, "{\"error\":\"enter a valid Sunshine host name or IP address\"}".into(), "application/json"); return;
        }
        if state.moonlight_pairing.is_some() {
            let status = moonlight_pair_status(state);
            if status["state"] == "waiting" {
                respond(request, 409, json(&serde_json::json!({"error":"Moonlight pairing is already in progress"})), "application/json"); return;
            }
        }
        let child = match moonlight_command(cfg).arg("pair").arg(host).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(child) => child,
            Err(error) => { respond(request, 500, json(&serde_json::json!({"error":format!("could not start Moonlight pairing with '{}': {error}", cfg.moonlight)})), "application/json"); return; }
        };
        state.moonlight_pair_result = None;
        state.moonlight_pairing = Some(MoonlightPairing { host: host.to_string(), executable: cfg.moonlight.clone(), child, verifying: false });
        respond(request, 202, json(&serde_json::json!({"ok":true, "host":host})), "application/json");
        return;
    }
    if method == Method::Post && path == "/sunshine" {
        let mut body = String::new();
        if request.as_reader().read_to_string(&mut body).is_err() { respond(request, 400, "{\"error\":\"invalid body\"}".into(), "application/json"); return; }
        let parsed: MirrorRequest = match serde_json::from_str(&body) { Ok(v) => v, Err(e) => { respond(request, 400, json(&serde_json::json!({"error": e.to_string()})), "application/json"); return; } };
        if parsed.host.trim().is_empty() { respond(request, 400, "{\"error\":\"host is required\"}".into(), "application/json"); return; }
        let template = if cfg.moonlight_args.is_empty() { vec!["stream".into(), "{host}".into(), "{app}".into()] } else { cfg.moonlight_args.clone() };
        let host_args = expand(&template, &parsed.host, "{host}");
        let args = expand(&host_args, &parsed.app, "{app}");
        let mut args = args;
        if !template.iter().any(|a| a.contains("{host}")) { args.push(parsed.host); }
        if !template.iter().any(|a| a.contains("{app}")) { args.push(parsed.app); }
        state.playback_epoch = state.playback_epoch.wrapping_add(1);
        state.playback_error = None;
        let _ = stop_current(state);
        state.queue.clear();
        if let Some(mut child) = state.mirror.take() { let _ = child.kill(); let _ = child.wait(); }
        match moonlight_command(cfg).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(child) => { state.mirror = Some(child); respond(request, 202, "{\"ok\":true}".into(), "application/json"); }
            Err(e) => respond(request, 500, json(&serde_json::json!({"error": format!("could not start Moonlight '{}': {e}", cfg.moonlight)})), "application/json"),
        }
        return;
    }
    respond(request, 404, "{\"error\":\"not found\"}".into(), "application/json");
}

fn advertised_listen_address(bind: &str) -> String {
    let Ok(address) = bind.parse::<SocketAddr>() else { return bind.to_string(); };
    if !address.ip().is_unspecified() {
        return format!("http://{address}");
    }
    if address.is_ipv4() {
        if let Ok(route_probe) = UdpSocket::bind("0.0.0.0:0") {
            if route_probe.connect("192.0.2.1:9").is_ok() {
                if let Ok(local) = route_probe.local_addr() {
                    if !local.ip().is_unspecified() {
                        return format!("http://{}:{} (bound to {})", local.ip(), address.port(), address);
                    }
                }
            }
        }
    }
    format!("{} (LAN address unavailable; use this host's IP)", address)
}

fn dashboard_url(bind: &str) -> String {
    let Ok(address) = bind.parse::<SocketAddr>() else { return format!("http://{bind}/"); };
    if !address.ip().is_unspecified() { return format!("http://{address}/"); }
    let ip = if address.is_ipv4() {
        UdpSocket::bind("0.0.0.0:0").ok().and_then(|socket| {
            socket.connect("192.0.2.1:9").ok()?;
            let local = socket.local_addr().ok()?.ip();
            (!local.is_unspecified()).then_some(local)
        }).unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
    } else { std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST) };
    format!("http://{}/", SocketAddr::new(ip, address.port()))
}

fn run_server(cfg: Config, shutdown: Option<Arc<AtomicBool>>) -> Result<Option<String>, String> {
    let _ = REMOTE_ORIGINS.set(RwLock::new(cfg.remote_origin_servers.clone()));
    let (cookies_browser, cookies_profile) = configured_cookie_source(&cfg);
    let _ = PLAYBACK_PREFERENCES.set(RwLock::new(PlaybackPreferences {
        default_screen: cfg.default_screen,
        default_quality: cfg.default_quality.clone(),
        subtitles: cfg.subtitles,
        preferred_language: cfg.preferred_language.clone(),
        cookies_browser,
        cookies_profile,
    }));
    let server = Server::http(&cfg.bind).map_err(|e| format!("could not listen on {}: {e}", cfg.bind))?;
    println!("mediabridge listening on {}", advertised_listen_address(&cfg.bind));
    let (player_error_tx, player_error_rx) = mpsc::channel();
    let (metadata_tx, metadata_rx) = mpsc::channel();
    let (autoplay_tx, autoplay_rx) = mpsc::channel();
    let mut state = State::default();
    state.player_error_tx = Some(player_error_tx);
    state.player_error_rx = Some(player_error_rx);
    state.metadata_tx = Some(metadata_tx);
    state.metadata_rx = Some(metadata_rx);
    state.autoplay_tx = Some(autoplay_tx);
    state.autoplay_rx = Some(autoplay_rx);
    while !shutdown.as_ref().is_some_and(|flag| flag.load(Ordering::Acquire)) {
        advance(&mut state, &cfg);
        if let Some(request) = server.recv_timeout(Duration::from_millis(200)).unwrap_or(None) { handle(request, &cfg, &mut state); }
        else { thread::yield_now(); }
        if state.restart_bind.is_some() { break; }
    }
    let restart_bind = state.restart_bind.take();
    drop(server);
    let _ = stop_current(&mut state);
    if let Some(mut child) = state.mirror.take() { terminate_child(&mut child); }
    Ok(restart_bind)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    let cfg = match config() { Ok(c) => c, Err(e) => { eprintln!("{e}"); std::process::exit(2); } };
    match run_server(cfg, None) {
        Ok(Some(_)) => match env::current_exe().and_then(|exe| Command::new(exe).spawn()) {
            Ok(_) => {}, Err(error) => { eprintln!("could not restart MediaBridge: {error}"); std::process::exit(1); }
        },
        Ok(None) => {},
        Err(error) => { eprintln!("{error}"); std::process::exit(1); }
    }
}

#[cfg(target_os = "linux")]
enum LinuxTrayEvent {
    Menu(tray_icon::menu::MenuEvent),
    Icon(tray_icon::TrayIconEvent),
}

#[cfg(target_os = "linux")]
struct LinuxTrayApp {
    shutdown: Arc<AtomicBool>,
    dashboard_url: String,
    tray: Option<tray_icon::TrayIcon>,
    dashboard_id: Option<tray_icon::menu::MenuId>,
    logs_id: Option<tray_icon::menu::MenuId>,
    quit_id: Option<tray_icon::menu::MenuId>,
}

#[cfg(target_os = "linux")]
impl LinuxTrayApp {
    fn open_dashboard(&self) {
        if let Err(error) = open_dashboard_in_librewolf(&self.dashboard_url) {
            append_linux_log(format!("Could not open dashboard in LibreWolf: {error}"));
        }
    }

    fn open_logs(&self) {
        if let Err(error) = Command::new("xdg-open").arg(linux_log_path()).spawn() {
            append_linux_log(format!("Could not open the MediaBridge log file: {error}"));
        }
    }
}

#[cfg(target_os = "linux")]
impl winit::application::ApplicationHandler<LinuxTrayEvent> for LinuxTrayApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.tray.is_some() { return; }
        use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
        use tray_icon::{Icon, TrayIconBuilder};
        let menu = Menu::new();
        let dashboard = MenuItem::new("Open Dashboard", true, None);
        let logs = MenuItem::new("Show logs", true, None);
        let quit = MenuItem::new("Quit MediaBridge", true, None);
        self.dashboard_id = Some(dashboard.id().clone());
        self.logs_id = Some(logs.id().clone());
        self.quit_id = Some(quit.id().clone());
        if let Err(error) = menu.append(&dashboard)
            .and_then(|_| menu.append(&PredefinedMenuItem::separator()))
            .and_then(|_| menu.append(&logs))
            .and_then(|_| menu.append(&PredefinedMenuItem::separator()))
            .and_then(|_| menu.append(&quit)) {
            append_linux_log(format!("Could not create tray menu: {error}"));
            event_loop.exit();
            return;
        }
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 2..14 {
            for x in 2..14 {
                let i = (y * 16 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[35, 166, 223, 255]);
            }
        }
        for (x, y) in [(6, 5), (7, 6), (8, 7), (9, 8), (8, 9), (7, 10), (6, 11)] {
            let i = (y * 16 + x) * 4;
            pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let icon = match Icon::from_rgba(pixels, 16, 16) {
            Ok(icon) => icon,
            Err(error) => {
                append_linux_log(format!("Could not create tray icon image: {error}"));
                event_loop.exit();
                return;
            }
        };
        match TrayIconBuilder::new().with_tooltip("MediaBridge")
            .with_menu(Box::new(menu)).with_icon(icon).with_menu_on_left_click(false).build() {
            Ok(tray) => self.tray = Some(tray),
            Err(error) => {
                append_linux_log(format!("Could not create Linux tray icon: {error}"));
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: LinuxTrayEvent) {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        match event {
            LinuxTrayEvent::Menu(event) if self.dashboard_id.as_ref().is_some_and(|id| id == &event.id) => self.open_dashboard(),
            LinuxTrayEvent::Menu(event) if self.logs_id.as_ref().is_some_and(|id| id == &event.id) => self.open_logs(),
            LinuxTrayEvent::Menu(event) if self.quit_id.as_ref().is_some_and(|id| id == &event.id) => {
                self.shutdown.store(true, Ordering::Release);
                event_loop.exit();
            }
            LinuxTrayEvent::Icon(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) => self.open_logs(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.shutdown.load(Ordering::Acquire) { event_loop.exit(); }
        else { event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_millis(200))); }
    }

    fn window_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _window_id: winit::window::WindowId, _event: winit::event::WindowEvent) {}
}

#[cfg(target_os = "linux")]
fn run_linux_server(cfg: Config) {
    let mut cfg = cfg;
    loop {
        match run_server(cfg, None) {
            Ok(Some(_)) => match config() {
                Ok(next) => cfg = next,
                Err(error) => { eprintln!("could not reload MediaBridge settings after restart: {error}"); break; }
            },
            Ok(None) => break,
            Err(error) => { eprintln!("{error}"); break; }
        }
    }
}

#[cfg(target_os = "linux")]
fn main() {
    initialize_linux_log();
    let cfg = match config() {
        Ok(cfg) => cfg,
        Err(error) => { append_linux_log(error); std::process::exit(2); }
    };
    let has_desktop_session = env::var_os("DISPLAY").is_some() || env::var_os("WAYLAND_DISPLAY").is_some();
    if !has_desktop_session {
        append_linux_log("No graphical desktop session detected; starting MediaBridge without a tray icon.".into());
        run_linux_server(cfg);
        return;
    }

    if let Err(error) = gtk::init() {
        append_linux_log(format!("Could not initialize GTK for the tray; running without a tray icon: {error}"));
        run_linux_server(cfg);
        return;
    }

    let event_loop = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        winit::event_loop::EventLoop::<LinuxTrayEvent>::with_user_event().build()
    })) {
        Ok(Ok(event_loop)) => event_loop,
        Ok(Err(error)) => {
            append_linux_log(format!("Could not start the desktop tray loop; running without a tray icon: {error}"));
            run_linux_server(cfg);
            return;
        }
        Err(_) => {
            append_linux_log("Desktop tray initialization panicked; running without a tray icon.".into());
            run_linux_server(cfg);
            return;
        }
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = Arc::clone(&shutdown);
    let server_thread = thread::spawn(move || {
        while !server_shutdown.load(Ordering::Acquire) {
            let server_cfg = match config() {
                Ok(cfg) => cfg,
                Err(error) => { append_linux_log(error); server_shutdown.store(true, Ordering::Release); break; }
            };
            match run_server(server_cfg, Some(Arc::clone(&server_shutdown))) {
                Ok(Some(_)) => continue,
                Ok(None) => break,
                Err(error) => { append_linux_log(error); server_shutdown.store(true, Ordering::Release); break; }
            }
        }
    });
    let dashboard_url = dashboard_url(&cfg.bind);
    let proxy = event_loop.create_proxy();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| { let _ = proxy.send_event(LinuxTrayEvent::Icon(event)); }));
    let proxy = event_loop.create_proxy();
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| { let _ = proxy.send_event(LinuxTrayEvent::Menu(event)); }));
    let mut app = LinuxTrayApp {
        shutdown: Arc::clone(&shutdown), dashboard_url, tray: None,
        dashboard_id: None, logs_id: None, quit_id: None,
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| event_loop.run_app(&mut app))) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => append_linux_log(format!("Linux tray event loop stopped: {error}")),
        Err(_) => append_linux_log("Linux tray event loop panicked; keeping MediaBridge running without a tray icon.".into()),
    }
    if app.tray.is_none() && !shutdown.load(Ordering::Acquire) {
        append_linux_log("The desktop has no usable tray backend; continuing with MediaBridge running without a tray icon.".into());
        // The tray event loop has exited, but the HTTP server is independent.
        // Keep the service alive headless until systemd (or another supervisor)
        // stops it; setting shutdown here would make the server exit too.
        let _ = server_thread.join();
        return;
    }
    shutdown.store(true, Ordering::Release);
    let _ = server_thread.join();
}

#[cfg(windows)]
enum TrayUiEvent {
    Menu(tray_icon::menu::MenuEvent),
    Icon(tray_icon::TrayIconEvent),
}

#[cfg(windows)]
struct WindowsTrayApp {
    shutdown: Arc<AtomicBool>,
    dashboard_url: String,
    tray: Option<tray_icon::TrayIcon>,
    open_dashboard_id: Option<tray_icon::menu::MenuId>,
    show_logs_id: Option<tray_icon::menu::MenuId>,
    quit_id: Option<tray_icon::menu::MenuId>,
    log_window: Option<Child>,
}

#[cfg(windows)]
impl WindowsTrayApp {
    fn open_dashboard(&mut self) {
        if let Err(error) = open_dashboard_in_librewolf(&self.dashboard_url) {
            append_windows_log(format!("Could not open dashboard at {}: {error}", self.dashboard_url));
        }
    }

    fn open_logs(&mut self) {
        if let Some(child) = self.log_window.as_mut() {
            if matches!(child.try_wait(), Ok(None)) { return; }
        }
        use std::os::windows::process::CommandExt;
        let path = windows_log_path().to_string_lossy().replace('\'', "''");
        let command = format!("Get-Content -LiteralPath '{path}' -Tail 100 -Wait");
        match Command::new("powershell.exe").args(["-NoExit", "-Command", &command])
            .creation_flags(0x00000010).spawn() {
            Ok(child) => self.log_window = Some(child),
            Err(error) => append_windows_log(format!("Could not open the log console: {error}")),
        }
    }
}

#[cfg(windows)]
impl winit::application::ApplicationHandler<TrayUiEvent> for WindowsTrayApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.tray.is_some() { return; }
        use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
        use tray_icon::{Icon, TrayIconBuilder};
        let menu = Menu::new();
        let open_dashboard = MenuItem::new("Open Dashboard", true, None);
        let show_logs = MenuItem::new("Show logs", true, None);
        let quit = MenuItem::new("Quit MediaBridge", true, None);
        self.open_dashboard_id = Some(open_dashboard.id().clone());
        self.show_logs_id = Some(show_logs.id().clone());
        self.quit_id = Some(quit.id().clone());
        if let Err(error) = menu.append(&open_dashboard)
            .and_then(|_| menu.append(&PredefinedMenuItem::separator()))
            .and_then(|_| menu.append(&show_logs))
            .and_then(|_| menu.append(&PredefinedMenuItem::separator()))
            .and_then(|_| menu.append(&quit)) {
            append_windows_log(format!("Could not create tray menu: {error}"));
            event_loop.exit();
            return;
        }
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 2..14 {
            for x in 2..14 {
                let i = (y * 16 + x) * 4;
                pixels[i..i + 4].copy_from_slice(&[35, 166, 223, 255]);
            }
        }
        for (x, y) in [(6, 5), (7, 6), (8, 7), (9, 8), (8, 9), (7, 10), (6, 11)] {
            let i = (y * 16 + x) * 4;
            pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let icon = match Icon::from_rgba(pixels, 16, 16) {
            Ok(icon) => icon,
            Err(error) => { append_windows_log(format!("Could not create tray icon image: {error}")); event_loop.exit(); return; }
        };
        match TrayIconBuilder::new().with_tooltip("MediaBridge")
            .with_menu(Box::new(menu)).with_icon(icon).with_menu_on_left_click(false).build() {
            Ok(tray) => self.tray = Some(tray),
            Err(error) => { append_windows_log(format!("Could not create system tray icon: {error}")); event_loop.exit(); }
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: TrayUiEvent) {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        match event {
            TrayUiEvent::Menu(event) if self.open_dashboard_id.as_ref().is_some_and(|id| id == &event.id) => self.open_dashboard(),
            TrayUiEvent::Menu(event) if self.show_logs_id.as_ref().is_some_and(|id| id == &event.id) => self.open_logs(),
            TrayUiEvent::Menu(event) if self.quit_id.as_ref().is_some_and(|id| id == &event.id) => {
                self.shutdown.store(true, Ordering::Release);
                if let Some(mut child) = self.log_window.take() { let _ = child.kill(); }
                event_loop.exit();
            }
            TrayUiEvent::Icon(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) => self.open_logs(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.shutdown.load(Ordering::Acquire) { event_loop.exit(); }
        else { event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_millis(200))); }
    }

    fn window_event(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop, _window_id: winit::window::WindowId, _event: winit::event::WindowEvent) {}
}

#[cfg(windows)]
fn main() {
    initialize_windows_log();
    let cfg = match config() {
        Ok(cfg) => cfg,
        Err(error) => { append_windows_log(error); return; }
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = Arc::clone(&shutdown);
    let restart_bind = Arc::new(Mutex::new(None));
    let server_restart_bind = Arc::clone(&restart_bind);
    let open_dashboard_url = dashboard_url(&cfg.bind);
    let server_thread = thread::spawn(move || {
        match run_server(cfg, Some(Arc::clone(&server_shutdown))) {
            Ok(Some(bind)) => {
                if let Ok(mut target) = server_restart_bind.lock() { *target = Some(bind); }
                server_shutdown.store(true, Ordering::Release);
            }
            Ok(None) => {},
            Err(error) => append_windows_log(error),
        }
    });
    let event_loop = match winit::event_loop::EventLoop::<TrayUiEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            append_windows_log(format!("Could not start Windows tray event loop: {error}"));
            shutdown.store(true, Ordering::Release);
            let _ = server_thread.join();
            return;
        }
    };
    let proxy = event_loop.create_proxy();
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event| { let _ = proxy.send_event(TrayUiEvent::Icon(event)); }));
    let proxy = event_loop.create_proxy();
    tray_icon::menu::MenuEvent::set_event_handler(Some(move |event| { let _ = proxy.send_event(TrayUiEvent::Menu(event)); }));
    let mut app = WindowsTrayApp { shutdown: Arc::clone(&shutdown), dashboard_url: open_dashboard_url, tray: None, open_dashboard_id: None, show_logs_id: None, quit_id: None, log_window: None };
    if let Err(error) = event_loop.run_app(&mut app) { append_windows_log(format!("Tray event loop stopped: {error}")); }
    shutdown.store(true, Ordering::Release);
    let _ = server_thread.join();
    let restarting = restart_bind.lock().ok().and_then(|mut target| target.take());
    if restarting.is_some() {
        match env::current_exe().and_then(|exe| Command::new(exe).spawn()) {
            Ok(_) => {}, Err(error) => append_windows_log(format!("Could not restart MediaBridge: {error}")),
        }
    }
}
