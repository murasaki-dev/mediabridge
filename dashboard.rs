use super::*;
use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex as CacheMutex, atomic::AtomicUsize};
use std::time::Instant;

include!(concat!(env!("OUT_DIR"), "/dashboard_assets.rs"));

const WALLPAPER_LIMIT: usize = 8 * 1024 * 1024;

fn asset(path: &str) -> Option<&'static [u8]> {
    ASSETS.binary_search_by_key(&path, |(name, _)| *name).ok().map(|index| ASSETS[index].1)
}

fn bytes(request: Request, data: &'static [u8], content_type: &str, cache: &str) {
    let response = Response::from_data(data)
        .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
        .with_header(Header::from_bytes("Cache-Control", cache).unwrap())
        .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap());
    let _ = request.respond(response);
}

pub(super) fn serve_asset(request: Request, path: &str) {
    let name = match path.trim_end_matches('/') {
        "" | "/index.html" | "/search" | "/chat" | "/mousepad" | "/appearance" | "/settings" => "/index.html",
        _ => path,
    };
    let Some(data) = asset(name) else {
        respond(request, 404, "Dashboard resource not found".into(), "text/plain; charset=utf-8");
        return;
    };
    let content_type = if name.ends_with(".html") { "text/html; charset=utf-8" }
        else if name.ends_with(".js") { "text/javascript; charset=utf-8" }
        else if name.ends_with(".css") { "text/css; charset=utf-8" }
        else { "application/octet-stream" };
    let cache = if name.starts_with("/_app/immutable/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    bytes(request, data, content_type, cache);
}

pub(super) fn is_asset_route(path: &str) -> bool {
    matches!(path.trim_end_matches('/'), "" | "/index.html" | "/search" | "/chat" | "/mousepad" | "/appearance" | "/settings")
        || path.starts_with("/_app/")
}

pub(super) fn embedded_settings(request: Request) {
    let head = concat!(
        "<style>", include_str!("dashboard-preview/svelte/static/settings-embed.css"), "</style>",
        "<script>", include_str!("dashboard-preview/svelte/static/settings-embed.js"), "</script></head>"
    );
    let markup = include_str!("web/index.html").replacen("</head>", head, 1);
    respond(request, 200, markup, "text/html; charset=utf-8");
}

pub(super) fn youtube_public(request: Request, search: bool) {
    let reply = |request, code, body: serde_json::Value| respond(request, code, json(&body), "application/json");
    let parameter = if search { "q" } else { "id" };
    let value = reqwest::Url::parse(&format!("http://localhost{}", request.url())).ok()
        .and_then(|url| url.query_pairs().find(|(key, _)| key == parameter).map(|(_, value)| value.trim().to_owned()));
    let Some(value) = value.filter(|value| {
        if search { !value.is_empty() && value.chars().count() <= 120 }
        else { value.len() == 11 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) }
    }) else {
        let error = if search { "Enter a search of 1–120 characters." } else { "Invalid YouTube video ID." };
        reply(request, 400, serde_json::json!({"error":error}));
        return;
    };
    let (path, query_key, source, ttl) = if search {
        ("results", "search_query", include_str!("dashboard-preview/svelte/obscura/youtube-search.js"), 120)
    } else {
        ("live_chat", "v", include_str!("dashboard-preview/svelte/obscura/youtube-live.js"), 30)
    };
    let mut url = reqwest::Url::parse(&format!("https://www.youtube.com/{path}")).unwrap();
    url.query_pairs_mut().append_pair(query_key, &value);
    let url = url.to_string();

    type PublicCache = CacheMutex<HashMap<String, (Instant, serde_json::Value)>>;
    static CACHE: OnceLock<PublicCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| CacheMutex::new(HashMap::new()));
    let saved = cache.lock().ok().and_then(|store| store.get(&url)
        .filter(|(time, _)| time.elapsed() < Duration::from_secs(ttl)).map(|(_, data)| data.clone()));
    if let Some(saved) = saved { reply(request, 200, saved); return; }

    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    if ACTIVE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| (count < 2).then_some(count + 1)).is_err() {
        reply(request, 503, serde_json::json!({"error":"YouTube is handling other requests. Try again shortly."}));
        return;
    }
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) { ACTIVE.fetch_sub(1, Ordering::Release); }
    }
    let _permit = Permit;
    let result = recommendations::public_obscura_json(&url, source).ok().and_then(|data| {
        if search && data["available"] == true && data["videos"].is_array() {
            Some(serde_json::json!({"videos":data["videos"]}))
        } else if !search && data["live"].is_boolean() { Some(data) }
        else { None }
    });
    if let Some(result) = result {
        if let Ok(mut store) = cache.lock() {
            if store.len() >= 32 { store.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(120)); }
            if store.len() < 32 { store.insert(url, (Instant::now(), result.clone())); }
        }
        reply(request, 200, result);
    } else {
        let error = if search { "YouTube search could not load through Obscura." } else { "Could not check YouTube live chat." };
        reply(request, 502, serde_json::json!({"error":error}));
    }
}

fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
    else if bytes.starts_with(b"\xff\xd8\xff") { Some("image/jpeg") }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { Some("image/webp") }
    else { None }
}

fn wallpaper_path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let root = env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .ok_or("LOCALAPPDATA is unavailable")?.join("MediaBridge");
    #[cfg(not(windows))]
    let root = user_config_path()?.parent().ok_or("Missing configuration directory")?.to_owned();
    Ok(root.join("dashboard-wallpaper"))
}

pub(super) fn wallpaper(mut request: Request) {
    let reply = |request, code, message: &str| respond(request, code, message.into(), "text/plain; charset=utf-8");
    let path = match wallpaper_path() {
        Ok(path) => path,
        Err(_) => { reply(request, 500, "Could not locate wallpaper storage."); return; }
    };
    match *request.method() {
        Method::Get | Method::Head => match fs::read(&path) {
            Ok(data) => {
                if let Some(content_type) = image_type(&data) {
                    let response = Response::from_data(data)
                        .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
                        .with_header(Header::from_bytes("Cache-Control", "no-cache").unwrap())
                        .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap());
                    let _ = request.respond(response);
                } else { reply(request, 415, "Unsupported saved wallpaper."); }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(data) = asset("/wallpaper.png") {
                    bytes(request, data, "image/png", "no-cache");
                } else { reply(request, 204, ""); }
            }
            Err(_) => reply(request, 500, "Could not read wallpaper."),
        },
        Method::Put => {
            if request.body_length().is_some_and(|length| length > WALLPAPER_LIMIT) {
                reply(request, 413, "Wallpaper must be 8 MB or smaller.");
                return;
            }
            let mut data = Vec::new();
            if request.as_reader().take((WALLPAPER_LIMIT + 1) as u64).read_to_end(&mut data).is_err() {
                reply(request, 400, "Could not read wallpaper upload.");
                return;
            }
            if data.len() > WALLPAPER_LIMIT { reply(request, 413, "Wallpaper must be 8 MB or smaller."); return; }
            if image_type(&data).is_none() { reply(request, 415, "Choose a PNG, JPEG, or WebP image."); return; }
            let temporary = path.with_extension(format!("{:016x}.tmp", rand::random::<u64>()));
            let result = fs::create_dir_all(path.parent().unwrap())
                .and_then(|_| fs::OpenOptions::new().create_new(true).write(true).open(&temporary))
                .and_then(|mut file| file.write_all(&data))
                .and_then(|_| fs::rename(&temporary, &path));
            if result.is_err() {
                let _ = fs::remove_file(temporary);
                reply(request, 500, "Could not save wallpaper.");
            } else { reply(request, 204, ""); }
        }
        Method::Delete => match fs::remove_file(path) {
            Ok(()) => reply(request, 204, ""),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => reply(request, 204, ""),
            Err(_) => reply(request, 500, "Could not reset wallpaper."),
        },
        _ => reply(request, 405, "Method unavailable."),
    }
}

#[test]
fn embedded_dashboard_has_spa_shell_and_assets() {
    assert!(asset("/index.html").is_some());
    assert!(ASSETS.iter().any(|(path, _)| path.starts_with("/_app/immutable/") && path.ends_with(".js")));
    assert!(is_asset_route("/mousepad/"));
    assert!(is_asset_route("/chat"));
    assert!(is_asset_route("/search"));
    assert!(!is_asset_route("/watch"));
    assert!(!is_asset_route("/settings/mouse"));
    assert_eq!(image_type(b"RIFF1234WEBP"), Some("image/webp"));
    assert_eq!(image_type(b"<svg>"), None);
}
