use super::*;
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Recommendations)>>> = OnceLock::new();
static PROFILE_LOCK: Mutex<()> = Mutex::new(());
static TWITCH_PROFILE_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn video_id(url: &str) -> Option<String> {
    youtube_url(url)
        .then(|| youtube_video_id(url))
        .flatten()
        .filter(|id| valid_id(id))
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn profile() -> Result<PathBuf, String> {
    browser_profile("obscura-profile")
}
fn browser_profile(name: &str) -> Result<PathBuf, String> {
    #[cfg(windows)]
    let root = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
        .join("MediaBridge");
    #[cfg(not(windows))]
    let root = user_config_path()?
        .parent()
        .ok_or("missing configuration directory")?
        .to_owned();
    let profile = root.join(name);
    std::fs::create_dir_all(&profile)
        .map_err(|e| format!("could not create the Obscura profile: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("could not protect the Obscura profile: {e}"))?;
    }
    Ok(profile)
}

fn youtube_cookies(export: &str) -> Vec<serde_json::Value> {
    export.lines().filter_map(|line| {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        let fields: Vec<_> = line.splitn(7, '\t').collect();
        let [domain, include_subdomains, path, secure, expires, name, value] = fields.as_slice() else { return None };
        let domain = domain.trim_start_matches('.').to_ascii_lowercase();
        if domain != "youtube.com" && !domain.ends_with(".youtube.com") { return None; }
        let expires = expires.parse::<i64>().ok().filter(|expires| *expires > 0);
        // yt-dlp may omit the HttpOnly marker in exports. Keep imported cookies hidden from page scripts.
        Some(serde_json::json!({
            "name": name, "value": value, "domain": domain, "path": path,
            "secure": *secure == "TRUE", "httpOnly": true, "hostOnly": *include_subdomains != "TRUE",
            "expires": expires
        }))
    }).collect()
}

pub(super) fn import_librewolf() -> Result<(), String> {
    let _guard = PROFILE_LOCK
        .try_lock()
        .map_err(|_| "a recommendation lookup is already running")?;
    let spec = cookie_browser_spec("librewolf", None)?
        .ok_or("Sign in to YouTube in MediaBridge's managed LibreWolf first")?;
    let temporary = cookie_check_temp_dir()?;
    let export = temporary.0.join("cookies.txt");
    let mut command = media_tool_command("yt-dlp");
    // yt-dlp writes the jar before reporting the missing URL. Read the file, not its exit status.
    command
        .args([
            "--ignore-config",
            "--cookies-from-browser",
            &spec,
            "--cookies",
        ])
        .arg(&export);
    let _ = run_cancellable_command(command, &AtomicBool::new(false))?;
    let contents = std::fs::read_to_string(&export).map_err(
        |_| "Could not export LibreWolf cookies. Sign in and close LibreWolf before retrying.",
    )?;
    let cookies = youtube_cookies(&contents);
    if !cookies.iter().any(|cookie| {
        matches!(
            cookie["name"].as_str(),
            Some("SAPISID" | "__Secure-3PAPISID" | "__Secure-1PAPISID")
        )
    }) {
        return Err("No YouTube sign-in cookies were found in managed LibreWolf".into());
    }
    let dir = profile()?;
    let result = save_cookie_profile(&dir, &cookies);
    drop(_guard);
    if result.is_ok() {
        if let Some(cache) = CACHE.get().and_then(|cache| cache.lock().ok()).as_mut() { cache.clear(); }
        subscription_cache(SubscriptionFeed::YouTube).invalidate();
        invalidate_home();
    }
    result
}

fn save_cookie_profile(dir: &Path, cookies: &[serde_json::Value]) -> Result<(), String> {
    let staging = dir.join("cookies.import");
    let mut file = std::fs::File::create(&staging)
        .map_err(|e| format!("could not stage Obscura cookies: {e}"))?;
    let result = serde_json::to_writer(&mut file, cookies)
        .map_err(|e| format!("could not serialize Obscura cookies: {e}"));
    drop(file);
    let result = result.and_then(|_| {
        std::fs::rename(&staging, dir.join("cookies.json"))
            .map_err(|e| format!("could not save Obscura cookies: {e}"))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(staging);
    }
    result
}

pub(super) fn import_twitch_librewolf() -> Result<(), String> {
    let _guard = TWITCH_PROFILE_LOCK.try_lock().map_err(|_| "Twitch is loading. Try the import again shortly")?;
    let spec = cookie_browser_spec("librewolf", None)?
        .ok_or("Sign in to Twitch in MediaBridge's managed LibreWolf first")?;
    let path = spec.strip_prefix("firefox:").ok_or("Could not find the managed LibreWolf profile")?;
    let cookies = read_twitch_cookies(&Path::new(path).join("cookies.sqlite"))?;
    if !cookies.iter().any(|cookie| cookie["name"] == "auth-token") {
        return Err("No Twitch sign-in cookie was found in managed LibreWolf's default profile".into());
    }
    save_cookie_profile(&browser_profile("obscura-twitch-profile")?, &cookies)?;
    subscription_cache(SubscriptionFeed::Twitch).invalidate();
    subscription_cache(SubscriptionFeed::TwitchLive).invalidate();
    Ok(())
}

fn read_twitch_cookies(path: &Path) -> Result<Vec<serde_json::Value>, String> {
    let read = |path: &Path| {
        let database = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        database.busy_timeout(Duration::from_millis(250))?;
        twitch_cookies(&database)
    };
    let error = match read(path) {
        Ok(cookies) => return Ok(cookies),
        Err(error) => error,
    };
    if matches!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)) {
        // Copy the WAL with the database so SQLite can read committed cookies
        // that LibreWolf has not checkpointed yet. Let SQLite regenerate the SHM file.
        let snapshot = cookie_check_temp_dir()?;
        let snapshot_db = snapshot.0.join("cookies.sqlite");
        let wal = path.with_file_name("cookies.sqlite-wal");
        let snapshot_wal = snapshot_db.with_file_name("cookies.sqlite-wal");
        let stamp = |path: &Path| {
            std::fs::metadata(path).and_then(|stat| Ok((stat.len(), stat.modified()?))).ok()
        };
        for _ in 0..3 {
            let before = (stamp(path), stamp(&wal));
            std::fs::copy(path, &snapshot_db).map_err(|error| format!("Could not snapshot the LibreWolf cookie database: {error}"))?;
            if before.1.is_some() {
                if let Err(error) = std::fs::copy(&wal, &snapshot_wal) {
                    if error.kind() == std::io::ErrorKind::NotFound { continue; }
                    return Err(format!("Could not snapshot the LibreWolf cookie journal: {error}"));
                }
            } else if snapshot_wal.is_file() {
                std::fs::remove_file(&snapshot_wal).map_err(|error| format!("Could not replace the LibreWolf cookie journal snapshot: {error}"))?;
            }
            if before == (stamp(path), stamp(&wal)) {
                return read(&snapshot_db).map_err(|error| format!("Could not read the LibreWolf cookie snapshot: {error}"));
            }
        }
        return Err("LibreWolf was updating cookies during the check. Try again, or quit LibreWolf and check from another browser on this host.".into());
    }
    Err(format!("Could not read Twitch cookies from LibreWolf: {error}"))
}

fn twitch_cookies(database: &rusqlite::Connection) -> rusqlite::Result<Vec<serde_json::Value>> {
    // Preserve HttpOnly exactly. Twitch's own web client may read cookies that LibreWolf marks script-readable.
    let schema: u32 = database.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    // Firefox's schema 16 migrated cookie expiry from Unix seconds to milliseconds.
    let expiry_divisor = if schema >= 16 { 1_000 } else { 1 };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)
        .unwrap_or_default().as_secs();
    let mut statement = database.prepare(
        "SELECT host, name, value, path, expiry / ?1, COALESCE(isSecure, 0), COALESCE(isHttpOnly, 1)
         FROM moz_cookies
         WHERE (host = 'twitch.tv' OR host LIKE '%.twitch.tv') AND COALESCE(originAttributes, '') = ''
         AND (expiry = 0 OR expiry / ?1 > ?2)"
    )?;
    let rows = statement.query_map(rusqlite::params![expiry_divisor, now], |row| {
        let host: String = row.get(0)?;
        let expiry: i64 = row.get(4)?;
        Ok(serde_json::json!({
            "domain":host.trim_start_matches('.'), "hostOnly": !host.starts_with('.'),
            "name": row.get::<_,String>(1)?, "value": row.get::<_,String>(2)?, "path": row.get::<_,String>(3)?,
            "expires":(expiry > 0).then_some(expiry), "secure":row.get::<_,bool>(5)?, "httpOnly":row.get::<_,bool>(6)?,
        }))
    })?;
    rows.collect()
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Video {
    pub(super) title: Option<String>,
    pub(super) url: String,
    pub(super) uploader: Option<String>,
    pub(super) published: Option<String>,
    pub(super) avatar_url: Option<String>,
    pub(super) duration_seconds: Option<u64>,
}

#[derive(Clone, Serialize)]
pub(super) struct Recommendations {
    pub(super) seed_video_id: Option<String>,
    pub(super) signed_in: bool,
    pub(super) videos: Vec<Video>,
}

const EXTRACT: &str = r#"(() => {
    const contents = globalThis.ytInitialData?.contents;
    const browse = contents?.twoColumnBrowseResultsRenderer || contents?.singleColumnBrowseResultsRenderer;
    const tab = browse?.tabs?.find(tab => tab.tabRenderer?.selected)?.tabRenderer;
    const root = location.pathname === '/watch' ? contents?.twoColumnWatchNextResults?.secondaryResults : tab?.content || browse?.sectionListRenderer || contents?.sectionListRenderer;
    const text = value => value?.content || value?.simpleText || value?.runs?.map(run => run.text).join('') || null;
    function duration(value) {
        if (!/^\d+(?::\d{2}){1,2}$/.test(value || '')) return null;
        return value.split(':').reduce((total, part) => total * 60 + Number(part), 0);
    }
    const seen = new Set();
    const videos = [];
    const promoted = /^(?:ad(?:Slot|Placement|Video|Display|Badge)|.*(?:promoted|sponsored|inFeedAd|inlineAd)|displayAd|searchPyv|inStreamAd)/i;
    function visit(node) {
        if (!node || typeof node !== 'object') return;
        const card = node.lockupViewModel || node.compactVideoRenderer || node.videoRenderer;
        if (card) {
            if (Object.keys(card).some(key => promoted.test(key))) return;
            const endpoint = (card.navigationEndpoint || card.rendererContext?.commandContext?.onTap?.innertubeCommand)?.watchEndpoint;
            const id = endpoint?.videoId;
            if (/^[A-Za-z0-9_-]{11}$/.test(id || '') && !endpoint.playlistId && !seen.has(id)) {
                const title = card.metadata?.lockupMetadataViewModel?.title?.content
                    || card.title?.simpleText || card.title?.runs?.map(run => run.text).join('');
                if (typeof title !== 'string' || !title.trim()) return;
                seen.add(id);
                const rows = card.metadata?.lockupMetadataViewModel?.metadata?.contentMetadataViewModel?.metadataRows || [];
                const parts = rows.flatMap(row => (row.metadataParts || []).map(part => text(part.text)));
                const overlays = card.contentImage?.thumbnailViewModel?.overlays || [];
                const lengths = overlays.flatMap(overlay => overlay.thumbnailBottomOverlayViewModel?.badges || [])
                    .map(badge => duration(badge.thumbnailBadgeViewModel?.text));
                const legacyAvatar = card.channelThumbnailSupportedRenderers?.channelThumbnailWithLinkRenderer?.thumbnail?.thumbnails
                    || card.channelThumbnail?.thumbnails;
                const image = card.metadata?.lockupMetadataViewModel?.image;
                const avatar = image?.decoratedAvatarViewModel?.avatar?.avatarViewModel?.image?.sources
                    || image?.avatarViewModel?.image?.sources;
                const icon = (legacyAvatar || avatar)?.at(-1)?.url;
                videos.push({id, title: title || null,
                    avatar_url: typeof icon === 'string' ? icon.replace(/^\/\//, 'https://') : null,
                    uploader: text(card.ownerText || card.shortBylineText) || parts[0] || null,
                    published: text(card.publishedTimeText) || parts.find(part => /ago$/i.test(part || '')) || null,
                    duration_seconds: duration(text(card.lengthText)) ?? lengths.find(value => value !== null) ?? null
                });
            }
            return;
        }
        for (const [kind, child] of Object.entries(node)) {
            if (!promoted.test(kind)) visit(child);
        }

    }
    visit(root);
    for (const batch of globalThis.mediaBridgeContinuationBatches || []) visit(batch);
    return {signed_in: globalThis.ytcfg?.get?.('LOGGED_IN') === true, available: !!root, videos};
})()"#;

pub(super) fn lookup(seed: String, cancel: &AtomicBool) -> Result<Recommendations, String> {
    lookup_feed(Some(seed), cancel)
}

pub(super) fn lookup_feed(seed: Option<String>, cancel: &AtomicBool) -> Result<Recommendations, String> {
    let url = seed.as_ref().map_or_else(|| "https://www.youtube.com/".to_owned(),
        |id| format!("https://www.youtube.com/watch?v={id}"));

    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().map_err(|_| "recommendation cache unavailable")?;
    if let Some((_, result)) = cache.get(&url).filter(|(at, _)| at.elapsed() < Duration::from_secs(60)) {
        return Ok(result.clone());
    }
    if cancel.load(Ordering::Acquire) { return Err("recommendation lookup cancelled".into()); }
    let result = lookup_url(&url, seed, cancel)?;
    if cache.len() >= 16 { cache.clear(); }
    cache.insert(url, (Instant::now(), result.clone()));
    Ok(result)
}

fn obscura_command() -> Command {
    #[cfg(target_os = "linux")]
    if let Some(path) = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("obscura")))
        .filter(|path| path.is_file())
    {
        return Command::new(path);
    }
    media_tool_command("obscura")
}

fn obscura_feed(url: &str, script: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let _guard = PROFILE_LOCK.lock().map_err(|_| "recommendation profile unavailable")?;
    if cancel.load(Ordering::Acquire) { return Err("recommendation lookup cancelled".into()); }
    fetch_obscura(url, script, &profile()?, "0", cancel)
}

fn fetch_obscura(url: &str, script: &str, storage: &Path, wait: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
    let mut command = obscura_command();
    command.env("RUST_LOG", "off");
    command
        .arg("fetch")
        .arg("--storage-dir")
        .arg(storage)
        .args([
            url,
            "--quiet",
            "--timeout",
            "15",
            "--wait-until",
            "domcontentloaded",
            "--wait",
            wait,
            "--eval",
            script,
        ]);
    let output = run_cancellable_command(command, cancel).map_err(|e| format!("Obscura: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Obscura could not load the page ({})",
            output.status
        ));
    }
    Ok(output.stdout)
}

#[cfg(feature = "native-dashboard")]
pub(super) fn public_obscura_json(url: &str, script: &str) -> Result<serde_json::Value, String> {
    let storage = cookie_check_temp_dir()?;
    let data = fetch_obscura(url, script, &storage.0, "0", &AtomicBool::new(false))?;
    serde_json::from_slice(&data).map_err(|_| "Obscura returned invalid data".into())
}

fn lookup_url(url: &str, seed: Option<String>, cancel: &AtomicBool) -> Result<Recommendations, String> {
    let output = obscura_feed(url, EXTRACT, cancel)?;
    parse_feed(&output, seed, 20, url.ends_with("/feed/history"))
}

fn parse_feed(output: &[u8], seed: Option<String>, limit: usize, history: bool) -> Result<Recommendations, String> {
    #[derive(Deserialize)]
    struct Candidate {
        id: String,
        title: Option<String>,
        uploader: Option<String>,
        published: Option<String>,
        avatar_url: Option<String>,
        duration_seconds: Option<u64>,
    }
    #[derive(Deserialize)]
    struct WatchData {
        signed_in: bool,
        available: bool,
        videos: Vec<Candidate>,
    }
    let data: WatchData = serde_json::from_slice(output)
        .map_err(|_| "Obscura returned invalid recommendation data")?;
    if !data.available && (data.signed_in || !history) {
        return Err("YouTube did not provide recommendations for this feed".into());
    }
    let videos = data
        .videos
        .into_iter()
        .filter(|video| seed.as_deref() != Some(video.id.as_str()) && valid_id(&video.id))
        .take(limit)
        .map(|video| Video {
            title: video.title,
            uploader: video.uploader,
            published: video.published,
            avatar_url: video.avatar_url,
            duration_seconds: video.duration_seconds,
            url: format!("https://www.youtube.com/watch?v={}", video.id),
        })
        .collect();
    Ok(Recommendations {
        seed_video_id: seed,
        signed_in: data.signed_in,
        videos,
    })
}

const CONTINUE_HOME: &str = r#"async (batchLimit) => {
    function continuation(node) {
        if (!node || typeof node !== 'object') return null;
        const item = node.continuationItemRenderer;
        if (item) return item.continuationEndpoint?.continuationCommand?.token
            || item.button?.buttonRenderer?.command?.continuationCommand?.token || null;
        for (const [key, child] of Object.entries(node).reverse()) {
            if (/^(?:ad|promoted|sponsored)/i.test(key)) continue;
            const token = continuation(child);
            if (token) return token;
        }
        return null;
    }
    const context = globalThis.ytcfg?.get?.('INNERTUBE_CONTEXT');
    const tabs = globalThis.ytInitialData?.contents?.twoColumnBrowseResultsRenderer?.tabs;
    let root = tabs?.find(tab => tab.tabRenderer?.selected)?.tabRenderer?.content;
    const used = new Set();
    const batches = [];
    for (let i = 0; context && i < batchLimit; i++) {
        const token = continuation(root);
        if (!token || used.has(token)) break;
        used.add(token);
        try {
            const response = await fetch('https://www.youtube.com/youtubei/v1/browse', {
                method:'POST', credentials:'same-origin',
                headers:{'Content-Type':'application/json','X-YouTube-Client-Name':'1','X-YouTube-Client-Version':context.client?.clientVersion || ''},
                body:JSON.stringify({context, continuation:token}),
            });
            if (!response.ok) break;
            const data = await response.json();
            root = data.onResponseReceivedActions || data.onResponseReceivedEndpoints || data.onResponseReceivedCommands || data.continuationContents;
            if (!root) break;
            batches.push(root);
        } catch { break; }
    }
    globalThis.mediaBridgeContinuationBatches = batches;
}"#;

fn expanded_home(cancel: &AtomicBool, publish_initial: bool, extend: bool) -> Result<Recommendations, String> {
    use rand::{Rng, distributions::Alphanumeric};
    use tungstenite::client::IntoClientRequest;
    let _guard = PROFILE_LOCK.lock().map_err(|_| "recommendation profile unavailable")?;
    if cancel.load(Ordering::Acquire) { return Err("YouTube Home refresh cancelled".into()); }
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "could not reserve Obscura port")?;
    let port = listener.local_addr().map_err(|_| "could not reserve Obscura port")?.port();
    drop(listener);
    let token: String = rand::rngs::OsRng.sample_iter(&Alphanumeric).take(48).map(char::from).collect();
    let mut command = obscura_command();
    command.args(["serve", "--host", "127.0.0.1", "--port", &port.to_string(), "--quiet"])
        .arg("--storage-dir").arg(profile()?).env("OBSCURA_CDP_TOKEN", &token).env("RUST_LOG","off")
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    isolate_process_group(&mut command);
    let mut child = command.spawn().map_err(|_| "could not start Obscura for the full Home feed")?;
    let result = (|| {
        let base = format!("127.0.0.1:{port}");
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(1)).no_proxy().build()
            .map_err(|_| "could not initialize Obscura discovery")?;
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if cancel.load(Ordering::Acquire) { return Err("YouTube Home refresh cancelled".into()); }
            if client.get(format!("http://{base}/json/version")).bearer_auth(&token).send()
                .is_ok_and(|response| response.status().is_success()) { break; }
            if Instant::now() >= deadline || child.try_wait().ok().flatten().is_some() {
                return Err("Obscura DevTools did not start".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        let mut request = format!("ws://{base}/devtools/browser").into_client_request()
            .map_err(|_| "could not create Obscura DevTools request")?;
        request.headers_mut().insert("Authorization", format!("Bearer {token}").parse()
            .map_err(|_| "could not create Obscura authorization")?);
        let stream = TcpStream::connect_timeout(&SocketAddr::from(([127,0,0,1],port)), Duration::from_secs(2))
            .map_err(|_| "could not connect to Obscura DevTools")?;
        stream.set_read_timeout(Some(Duration::from_millis(250))).map_err(|_| "could not set Obscura timeout")?;
        stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(|_| "could not set Obscura timeout")?;
        let (mut socket, _) = tungstenite::client(request, stream).map_err(|_| "could not connect to Obscura DevTools")?;
        let deadline = Instant::now() + Duration::from_secs(24);
        let mut id = 0u64;
        let mut call = |method: &str, params: serde_json::Value, session: Option<&str>| -> Result<serde_json::Value, String> {
            id += 1;
            let mut request = serde_json::json!({"id":id,"method":method,"params":params});
            if let Some(session) = session { request["sessionId"] = serde_json::json!(session); }
            socket.send(Message::Text(request.to_string())).map_err(|_| "could not send Obscura command")?;
            while Instant::now() < deadline && !cancel.load(Ordering::Acquire) {
                match socket.read() {
                    Ok(Message::Text(text)) => {
                        let reply: serde_json::Value = serde_json::from_str(&text).map_err(|_| "invalid Obscura response")?;
                        if reply["id"] != id { continue; }
                        if reply.get("error").is_some() || reply["result"].get("exceptionDetails").is_some() {
                            return Err(format!("Obscura could not complete {method}"));
                        }
                        return Ok(reply["result"].clone());
                    }
                    Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(_) | Ok(Message::Close(_)) => return Err("Obscura DevTools disconnected".into()),
                    _ => {}
                }
            }
            Err("YouTube Home refresh timed out or was cancelled".into())
        };
        let target = call("Target.createTarget", serde_json::json!({"url":"about:blank"}), None)?;
        let session = call("Target.attachToTarget", serde_json::json!({"targetId":target["targetId"],"flatten":true}), None)?;
        let session = session["sessionId"].as_str().ok_or("Obscura did not create a session")?;
        call("Page.navigate",serde_json::json!({"url":HOME_URL}),Some(session))?;
        let evaluate = |expression: &str, await_promise: bool| serde_json::json!({
            "expression":expression, "returnByValue":true, "awaitPromise":await_promise,
        });
        let initial = call("Runtime.evaluate",evaluate(EXTRACT,false),Some(session))?;
        let first = parse_feed(&serde_json::to_vec(&initial["result"]["value"]).map_err(|_| "invalid Home feed")?,None,if extend { EXTENDED_LIMIT } else { LONG_LIMIT },false)?;
        if publish_initial && !cancel.load(Ordering::Acquire) {
            let (mutex, ready) = home_cache();
            if let Ok(mut home) = mutex.lock() {
                if home.current.is_none() && !cancel.load(Ordering::Acquire) { publish_home(&mut home, first.clone()); }
                ready.notify_all();
            }
        }
        if !first.signed_in || call("Runtime.evaluate",evaluate(&format!("({CONTINUE_HOME})({})", if extend { 12 } else { 7 }),true),Some(session)).is_err() {
            return Ok(first);
        }
        let expanded = call("Runtime.evaluate",evaluate(EXTRACT,false),Some(session))?;
        parse_feed(&serde_json::to_vec(&expanded["result"]["value"]).map_err(|_| "invalid Home feed")?,None,if extend { EXTENDED_LIMIT } else { LONG_LIMIT },false)
    })();
    terminate_child(&mut child);
    result
}
const HOME_URL: &str = "https://www.youtube.com/";
pub(super) const SHORT_PAGE_SIZE: usize = 5;
pub(super) const LONG_LIMIT: usize = 100;
const EXTENDED_LIMIT: usize = 200;

#[derive(Default)]
struct HomeCache {
    current: Option<(u64, Recommendations)>,
    previous: Option<(u64, Recommendations)>,
    revision: u64,
    interval_minutes: u64,
    next_refresh: Option<Instant>,
    job: Option<(bool, Arc<AtomicBool>)>,
    error: Option<String>,
}
static HOME: OnceLock<(Mutex<HomeCache>, Condvar)> = OnceLock::new();

fn home_cache() -> &'static (Mutex<HomeCache>, Condvar) {
    HOME.get_or_init(|| (Mutex::new(HomeCache::default()), Condvar::new()))
}

fn invalidate_home() {
    let (mutex, ready) = home_cache();
    if let Ok(mut home) = mutex.lock() {
        if let Some((_, cancel)) = &home.job { cancel.store(true, Ordering::Release); }
        home.current = None;
        home.previous = None;
        home.error = None;
        home.next_refresh = None;
        ready.notify_all();
    }
}

fn publish_home(home: &mut HomeCache, feed: Recommendations) {
    home.revision = home.revision.wrapping_add(1);
    home.previous = home.current.take();
    home.current = Some((home.revision, feed));
}

fn start_home_job(home: &mut HomeCache, full: bool, extend: bool, scheduled: bool) {
    if home.job.is_some() { return; }
    let cancel = Arc::new(AtomicBool::new(false));
    home.job = Some((full, cancel.clone()));
    home.error = None;
    let cold = home.current.is_none();
    thread::spawn(move || {
        let (mutex, ready) = home_cache();
        let mut result = if full { expanded_home(&cancel, cold, extend) } else { lookup_url(HOME_URL, None, &cancel) };
        if full && result.is_err() && cold && !cancel.load(Ordering::Acquire)
            && mutex.lock().is_ok_and(|home| home.current.is_none())
        {
            result = lookup_url(HOME_URL, None, &cancel);
        }
        if let Ok(mut home) = mutex.lock() {
            home.job = None;
            if !cancel.load(Ordering::Acquire) {
                match result {
                    Ok(mut feed) => {
                        if extend {
                            if let Some((_, current)) = &home.current {
                                let known: HashSet<_> = current.videos.iter().map(|video| video.url.as_str()).collect();
                                let extra = feed.videos.into_iter().filter(|video| !known.contains(video.url.as_str()))
                                    .take(EXTENDED_LIMIT.saturating_sub(current.videos.len())).collect::<Vec<_>>();
                                feed = current.clone();
                                feed.videos.extend(extra);
                            }
                        }
                        // Preserve a populated feed when YouTube temporarily returns an empty page.
                        if home.current.is_none() || (!feed.videos.is_empty() && (!extend || home.current.as_ref().is_some_and(|(_, old)| feed.videos.len() > old.videos.len()))) { publish_home(&mut home, feed); }
                    }
                    Err(error) => home.error = Some(error),
                }
                home.next_refresh = Some(Instant::now() + Duration::from_secs(home.interval_minutes * 60));
            }
            ready.notify_all();
        }
        if scheduled && !cancel.load(Ordering::Acquire) {
            for feed in [SubscriptionFeed::YouTube, SubscriptionFeed::Twitch, SubscriptionFeed::TwitchLive] {
                if !mutex.lock().is_ok_and(|home| home.interval_minutes > 0) { break; }
                let cache = subscription_cache(feed);
                if let Ok(_guard) = cache.refresh.try_lock() { let _ = cache.fetch(feed); }
            }
        }
    });
}

pub(super) fn set_home_interval(minutes: u64) {
    if let Ok(mut home) = home_cache().0.lock() {
        if home.interval_minutes == minutes { return; }
        home.interval_minutes = minutes;
        home.next_refresh = None;
        if minutes == 0 {
            if let Some((true, cancel)) = &home.job { cancel.store(true, Ordering::Release); }
        }
    }
}

pub(super) fn tick_home() {
    if let Ok(mut home) = home_cache().0.lock() {
        if home.interval_minutes > 0 && home.next_refresh.is_none_or(|due| Instant::now() >= due) {
            start_home_job(&mut home, true, false, true);
        }
    }
}

pub(super) fn refresh_home() -> Result<(), &'static str> {
    let mut home = home_cache().0.lock().map_err(|_| "YouTube Home cache unavailable")?;
    if home.interval_minutes == 0 { return Err("Enable background Home refresh before requesting the full feed"); }
    start_home_job(&mut home, true, false, false);
    Ok(())
}

pub(super) fn refresh_short_home() -> Result<(), &'static str> {
    let mut home = home_cache().0.lock().map_err(|_| "YouTube Home cache unavailable")?;
    start_home_job(&mut home, false, false, false);
    Ok(())
}
pub(super) fn extend_home() -> Result<(), &'static str> {
    let mut home = home_cache().0.lock().map_err(|_| "YouTube Home cache unavailable")?;
    if home.interval_minutes == 0 { return Err("Full YouTube Home feed is disabled"); }
    if home.current.as_ref().is_some_and(|(_, feed)| feed.videos.len() >= EXTENDED_LIMIT) {
        return Err("The Home feed reached its cached limit");
    }
    start_home_job(&mut home, true, true, false);
    Ok(())
}
pub(super) fn home_page(snapshot: Option<u64>, offset: usize, limit: usize) -> Result<serde_json::Value, (u16, String)> {
    let (mutex, ready) = home_cache();
    let unavailable = || (503, "YouTube Home cache unavailable".to_string());
    let mut home = mutex.lock().map_err(|_| unavailable())?;
    if limit > 20 && home.interval_minutes == 0 { return Err((409, "Full YouTube Home feed is disabled".into())); }
    if home.current.is_none() && snapshot.is_none() && home.job.is_none() {
        let extended = home.interval_minutes > 0;
        start_home_job(&mut home, extended, false, false);
    }
    let (home, _) = ready.wait_timeout_while(home, Duration::from_secs(24), |state| {
        state.current.is_none() && state.job.is_some() && snapshot.is_none()
    }).map_err(|_| unavailable())?;
    let selected = if let Some(revision) = snapshot {
        home.current.iter().chain(home.previous.iter()).find(|(id, _)| *id == revision)
    } else { home.current.as_ref() };
    let Some((revision, feed)) = selected else {
        if snapshot.is_some() { return Err((409, "YouTube Home snapshot expired".into())); }
        return Err((if home.job.is_some() { 503 } else { 502 },
            home.error.clone().unwrap_or_else(|| "YouTube Home is still loading".into())));
    };

    let total = if limit == LONG_LIMIT { feed.videos.len().min(LONG_LIMIT) } else { feed.videos.len() };
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    Ok(serde_json::json!({
        "seed_video_id": null, "signed_in": feed.signed_in, "snapshot": revision,
        "videos": &feed.videos[start..end], "next_offset": (end < total).then_some(end),
        "can_extend": home.interval_minutes > 0 && feed.videos.len() < EXTENDED_LIMIT, "refreshing": home.job.is_some(), "long_enabled": home.interval_minutes > 0, "refresh_error": home.error,
    }))
}
pub(super) fn channel_icon(url: &str) -> Result<(String, Arc<[u8]>), String> {
    const MAX_SIZE: usize = 128 * 1024;
    const MAX_ENTRIES: usize = 128;
    type IconEntry = (Instant, String, Arc<[u8]>);
    static ICONS: OnceLock<Mutex<HashMap<String, IconEntry>>> = OnceLock::new();
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid channel icon URL")?;
    let allowed = match parsed.host_str() {
        Some("yt3.ggpht.com" | "yt3.googleusercontent.com") => parsed.path().starts_with('/'),
        Some("static-cdn.jtvnw.net") => parsed.path().starts_with("/jtv_user_pictures/"),
        _ => false,
    };
    if !allowed || parsed.scheme() != "https" || parsed.port().is_some() || !parsed.username().is_empty()
        || parsed.password().is_some() || parsed.fragment().is_some() || url.len() > 1024
    {
        return Err("Unsupported channel icon URL".into());
    }
    let key = parsed.to_string();
    let icons = ICONS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((_, kind, bytes)) = icons.lock().map_err(|_| "Channel icon cache unavailable")?
        .get(&key).filter(|(saved, _, _)| saved.elapsed() < Duration::from_secs(86400 * 7))
    {
        return Ok((kind.clone(), bytes.clone()));
    }
    if CLIENT.get().is_none() {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "Channel icon client unavailable")?;
        let _ = CLIENT.set(client);
    }
    let response = CLIENT.get().ok_or("Channel icon client unavailable")?.get(parsed).send()
        .map_err(|_| "Could not load channel icon")?;
    if !response.status().is_success() || response.content_length().is_some_and(|size| size > MAX_SIZE as u64) {
        return Err("Channel icon unavailable".into());
    }
    let kind = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .filter(|value| matches!(*value, "image/jpeg" | "image/png" | "image/webp" | "image/avif"))
        .ok_or("Unsupported channel icon format")?.to_owned();
    let mut bytes = Vec::new();
    response.take((MAX_SIZE + 1) as u64).read_to_end(&mut bytes).map_err(|_| "Could not read channel icon")?;
    if bytes.is_empty() || bytes.len() > MAX_SIZE { return Err("Invalid channel icon size".into()); }
    let bytes: Arc<[u8]> = bytes.into();
    let mut cache = icons.lock().map_err(|_| "Channel icon cache unavailable")?;
    if cache.len() >= MAX_ENTRIES {
        if let Some(oldest) = cache.iter().min_by_key(|(_, entry)| entry.0).map(|(key, _)| key.clone()) {
            cache.remove(&oldest);
        }
    }
    cache.insert(key,(Instant::now(),kind.clone(),bytes.clone()));
    Ok((kind,bytes))
}
const CHANNELS: &str = r#"(() => {
    const found = new Map();
    const label = value => value?.content || value?.simpleText || value?.runs?.map(run => run.text).join('');
    function walk(node) {
        if (!node || typeof node !== 'object') return;
        const card = node.channelRenderer || node.gridChannelRenderer || node.lockupViewModel;
        const endpoint = card?.navigationEndpoint || card?.rendererContext?.commandContext?.onTap?.innertubeCommand;
        const id = card?.channelId || endpoint?.browseEndpoint?.browseId;
        const title = label(card?.title) || label(card?.metadata?.lockupMetadataViewModel?.title);
        if (/^UC[A-Za-z0-9_-]{22}$/.test(id || '') && title) {
            const images = card.thumbnail?.thumbnails || card.contentImage?.avatarViewModel?.image?.sources || [];
            found.set(id,{id,title,thumbnail_url:images.at(-1)?.url?.replace(/^\/\//, 'https://') || null});
        }
        for (const child of Object.values(node)) walk(child);
    }
    walk(globalThis.ytInitialData?.contents);
    return {signed_in: globalThis.ytcfg?.get?.('LOGGED_IN') === true, channels:[...found.values()]};
})()"#;

fn youtube_channel_id(id: &str) -> bool {
    id.starts_with("UC") && id.len() == 24 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub(super) fn youtube_history() -> Result<Recommendations, String> {
    let result = lookup_url("https://www.youtube.com/feed/history", None, &AtomicBool::new(false))?;
    if !result.signed_in {
        return Err("Import your YouTube sign-in on the MediaBridge host first".into());
    }
    Ok(result)
}
const RECENT_CHANNELS: &str = r#"(() => {
    const found = [];
    const text = item => item?.content || item?.simpleText || item?.runs?.map(run => run.text).join('');
    const browse = globalThis.ytInitialData?.contents?.twoColumnBrowseResultsRenderer;
    const root = browse?.tabs?.find(tab => tab.tabRenderer?.selected)?.tabRenderer?.content;
    function visit(node) {
        if (!node || typeof node !== 'object' || found.length >= 100) return;
        const card = node.videoRenderer || node.gridVideoRenderer || node.lockupViewModel;
        if (card) {
            const watch = (card.navigationEndpoint || card.rendererContext?.commandContext?.onTap?.innertubeCommand)?.watchEndpoint;
            if (!watch?.videoId || watch.playlistId) return;
            const owner = card.ownerText || card.shortBylineText;
            const byline = card.metadata?.lockupMetadataViewModel?.metadata?.contentMetadataViewModel?.metadataRows?.[0]?.metadataParts?.[0]?.text;
            const run = (owner || byline)?.runs?.[0];
            const endpoint = run?.navigationEndpoint || run?.onTap?.innertubeCommand;
            const title = text(owner || byline);
            if (title) found.push({id:endpoint?.browseEndpoint?.browseId || null,title});
            return;
        }
        for (const [key, child] of Object.entries(node)) {
            if (!/^(?:adSlot|adPlacement|promoted|sponsored)/i.test(key)) visit(child);
        }
    }
    visit(root);
    return found;
})()"#;

fn youtube_subscribed_channels() -> Result<serde_json::Value, String> {
    let cancel = AtomicBool::new(false);
    let data = obscura_feed("https://www.youtube.com/feed/channels", CHANNELS, &cancel)?;
    let mut result: serde_json::Value = serde_json::from_slice(&data).map_err(|_| "YouTube returned invalid channel data")?;
    if result["signed_in"] != true { return Err("Import your YouTube sign-in on the MediaBridge host first".into()); }
    if let Ok(data) = obscura_feed("https://www.youtube.com/feed/subscriptions", RECENT_CHANNELS, &cancel) {
        if let (Ok(recent), Some(channels)) = (serde_json::from_slice::<Vec<serde_json::Value>>(&data), result["channels"].as_array_mut()) {
            for channel in channels {
                if let Some(rank) = recent.iter().position(|item| {
                    item["id"].as_str().map_or_else(|| item["title"] == channel["title"], |id| channel["id"] == id)
                }) {
                    channel["recent_rank"] = serde_json::json!(rank);
                }
            }
        }
    }
    Ok(result)
}
#[derive(Clone, Copy)]
enum SubscriptionFeed { YouTube, Twitch, TwitchLive }

#[derive(Default)]
struct SubscriptionCache {
    snapshot: Mutex<SubscriptionSnapshot>,
    refresh: Mutex<()>,
}

#[derive(Default)]
struct SubscriptionSnapshot {
    generation: u64,
    data: Option<(Instant, serde_json::Value)>,
}

fn subscription_cache(feed: SubscriptionFeed) -> &'static SubscriptionCache {
    static SUBSCRIPTIONS: OnceLock<[SubscriptionCache; 3]> = OnceLock::new();
    &SUBSCRIPTIONS.get_or_init(|| std::array::from_fn(|_| SubscriptionCache::default()))[feed as usize]
}

impl SubscriptionCache {
    fn invalidate(&self) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.generation = snapshot.generation.wrapping_add(1);
            snapshot.data = None;
        }
    }

    fn fetch(&self, feed: SubscriptionFeed) -> Result<serde_json::Value, String> {
        let unavailable = || "Subscription cache unavailable".to_string();
        let generation = self.snapshot.lock().map_err(|_| unavailable())?.generation;
        let data = match feed {
            SubscriptionFeed::YouTube => youtube_subscribed_channels(),
            SubscriptionFeed::Twitch => twitch_feed(None),
            SubscriptionFeed::TwitchLive => twitch_feed(Some("live")),
        }?;
        let mut snapshot = self.snapshot.lock().map_err(|_| unavailable())?;
        if snapshot.generation != generation {
            return Err("The account changed during refresh. Try again.".into());
        }
        snapshot.data = Some((Instant::now(), data.clone()));
        Ok(data)
    }

    fn get(&self, feed: SubscriptionFeed) -> Result<serde_json::Value, String> {
        let scheduled = home_cache().0.lock().is_ok_and(|home| home.interval_minutes > 0);
        let cached = |fresh_only: bool| {
            self.snapshot.lock().ok()?.data.as_ref()
                .filter(|(updated, _)| !fresh_only || scheduled || updated.elapsed() < Duration::from_secs(300))
                .map(|(_, data)| data.clone())
        };
        if let Some(data) = cached(true) { return Ok(data); }
        let _guard = self.refresh.lock().map_err(|_| "Subscription cache unavailable")?;
        if let Some(data) = cached(true) { return Ok(data); }
        self.fetch(feed).or_else(|error| cached(false).ok_or(error))
    }
}

pub(super) fn subscriptions(provider: &str, channel: Option<&str>) -> Result<serde_json::Value, String> {
    match (provider, channel) {
        ("youtube", None) => subscription_cache(SubscriptionFeed::YouTube).get(SubscriptionFeed::YouTube),
        ("youtube", Some(id)) if youtube_channel_id(id) => {
            let result = lookup_url(&format!("https://www.youtube.com/channel/{id}/videos"), None, &AtomicBool::new(false))?;
            serde_json::to_value(result).map_err(|_| "YouTube returned invalid video data".into())
        }
        ("twitch", None) => subscription_cache(SubscriptionFeed::Twitch).get(SubscriptionFeed::Twitch),
        ("twitch", Some("live")) => subscription_cache(SubscriptionFeed::TwitchLive).get(SubscriptionFeed::TwitchLive),
        ("twitch", Some(id)) if twitch_login(id) => twitch_feed(Some(id)),
        _ => Err("Invalid subscription provider or channel ID".into()),
    }
}

fn twitch_login(login: &str) -> bool {
    !login.is_empty() && login.len() <= 25 && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

const TWITCH_PAGE: &str = r#"(() => {
    const root = document.querySelector('main');
    const login = document.querySelector('[data-a-target="login-button"]');
    const account = document.querySelector('[data-a-target="user-menu-toggle"]');
    const signedIn = !!account && !login;
    const links = [...(root?.querySelectorAll('a[href]') || [])];
    const path = location.pathname;
    const loginName = path.split('/')[1];
    const channelName = root?.querySelector('h1')?.innerText?.trim() || loginName;
    const validLogin = value => /^[a-zA-Z0-9_]{1,25}$/.test(value || '');
    const reserved = /^(?:directory|downloads|jobs|turbo|inventory|settings|subscriptions|wallet|prime|search|videos|collections|products|drops)$/i;
    const image = (element, pattern) => [...(element?.querySelectorAll('img[src]') || [])]
        .map(img => img.currentSrc || img.src).find(url => pattern.test(url)) || null;
    const avatars = /static-cdn\.jtvnw\.net\/jtv_user_pictures\//i;
    const seen = new Set();
    const channels = [], videos = [];
    const mode = path.endsWith('/videos') ? 'videos' : path.endsWith('/live') ? 'live' : 'channels';

    if (mode === 'videos') {
        for (const link of links) {
            const id = link.pathname.match(/^\/videos\/(\d+)$/)?.[1];
            if (!id || seen.has(id)) continue;
            const matching = links.filter(other => other.pathname === link.pathname);
            const thumbnail = matching.find(a => a.dataset.aTarget === 'preview-card-image-link');
            const title = matching.find(a => a !== thumbnail && a.innerText.trim())?.innerText.trim();
            if (!title || !thumbnail) continue;
            seen.add(id);
            const details = thumbnail.innerText.split('\n').map(part => part.trim()).filter(Boolean);
            const duration = details.find(part => /^\d+(?::\d{2}){1,2}$/.test(part));
            videos.push({url:'https://www.twitch.tv/videos/' + id, title, uploader:channelName,
                thumbnail_url:image(thumbnail, /jtvnw\.net\//i),
                published:details.find(part => /ago$|yesterday|today/i.test(part)) || null,
                duration_seconds:duration ? duration.split(':').reduce((seconds, part) => seconds * 60 + Number(part), 0) : null});
            if (videos.length >= 20) break;
        }
    } else {
        for (const link of links) {
            const id = link.pathname.match(/^\/([A-Za-z0-9_]+)\/?$/)?.[1];
            if (!validLogin(id) || reserved.test(id) || seen.has(id)) continue;
            if (link.closest('[data-a-target*="recommend"],[data-test-selector*="recommend"]')) continue;
            const card = link.closest('article,[data-a-target="preview-card"],[data-test-selector="followed-channel-card"]')
                || link.parentElement?.parentElement;
            if (!card) continue;
            const title = card.querySelector('h2,h3,h4')?.innerText.trim() || link.innerText.trim().split('\n')[0];
            const icon = image(card,avatars);
            const thumbnail = image(card,/static-cdn\.jtvnw\.net\/previews-ttv\/live_user_/i);
            if (mode === 'channels' && !icon && !/^[A-Za-z0-9_]+$/i.test(title || '')) continue;
            if (mode === 'live' && !thumbnail && !card.querySelector('[data-a-target*="live-badge"]')) continue;
            seen.add(id);
            if (mode === 'channels') channels.push({id,title:title || id,thumbnail_url:icon});
            else videos.push({url:'https://www.twitch.tv/' + id,title:title || id,uploader:id,
                published:'Live now',avatar_url:icon,thumbnail_url:thumbnail});
            if (seen.size >= 200) break;
        }
    }
    return {signed_in:signedIn, available:!!root, channels, videos};
})()"#;

fn twitch_feed(channel: Option<&str>) -> Result<serde_json::Value, String> {
    let _guard = TWITCH_PROFILE_LOCK.lock().map_err(|_| "Twitch profile unavailable")?;
    let storage = browser_profile("obscura-twitch-profile")?;
    if !storage.join("cookies.json").is_file() {
        return Err("Import Twitch sign-in from managed LibreWolf in the MediaBridge host dashboard".into());
    }
    let url = match channel {
        None => "https://www.twitch.tv/directory/following/channels".to_owned(),
        Some("live") => "https://www.twitch.tv/directory/following/live".to_owned(),
        Some(login) => format!("https://www.twitch.tv/{login}/videos?filter=archives&sort=time"),
    };
    let output = fetch_obscura(&url,TWITCH_PAGE,&storage,"5",&AtomicBool::new(false))?;
    let result: serde_json::Value = serde_json::from_slice(&output).map_err(|_| "Twitch returned invalid page data")?;
    if result["available"] != true { return Err("Obscura could not render the Twitch page".into()); }
    if result["signed_in"] != true {
        return Err("Twitch did not recognize the imported session in Obscura. Check the sign-in in managed LibreWolf and import again".into());
    }
    Ok(result)
}

#[test]
fn cached_subscription_feed_is_available_during_refresh() {
    let cache = SubscriptionCache::default();
    let fixture = serde_json::json!({"signed_in": true, "channels": []});
    cache.snapshot.lock().unwrap().data = Some((Instant::now(), fixture.clone()));
    let _refresh = cache.refresh.lock().unwrap();
    assert_eq!(cache.get(SubscriptionFeed::YouTube).unwrap(), fixture);
    cache.invalidate();
    let snapshot = cache.snapshot.lock().unwrap();
    assert!(snapshot.data.is_none());
    assert_eq!(snapshot.generation, 1);
}

#[test]
fn imports_twitch_cookies_from_locked_librewolf_wal() {
    let folder = cookie_check_temp_dir().unwrap();
    let path = folder.0.join("cookies.sqlite");
    let browser = rusqlite::Connection::open(&path).unwrap();
    browser.execute_batch("
        CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT,
            expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, originAttributes TEXT);
        PRAGMA journal_mode = WAL;
        PRAGMA locking_mode = EXCLUSIVE;
        INSERT INTO moz_cookies VALUES ('.twitch.tv', 'auth-token', 'fixture', '/', 0, 1, 0, '');
    ").unwrap();
    let blocked = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    blocked.busy_timeout(Duration::ZERO).unwrap();
    assert_eq!(twitch_cookies(&blocked).unwrap_err().sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy));
    let cookies = read_twitch_cookies(&path).unwrap();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0]["name"], "auth-token");
    assert_eq!(cookies[0]["httpOnly"], false);
}

#[test]
fn imports_only_twitch_cookies_and_preserves_scope() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("
        CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT,
            expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, originAttributes TEXT);
        INSERT INTO moz_cookies VALUES
            ('.twitch.tv', 'auth-token', 'fixture', '/', unixepoch()+3600, 1, 0, ''),
            ('www.twitch.tv', 'local', 'fixture', '/directory', 0, 1, 1, ''),
            ('.twitch.tv', 'expired', 'fixture', '/', 1, 1, 1, ''),
            ('.not-twitch.tv', 'unrelated', 'fixture', '/', 0, 1, 1, ''),
            ('.twitch.tv', 'partitioned', 'fixture', '/', 0, 1, 1, '^partitionKey=fixture');
    ").unwrap();
    let cookies = twitch_cookies(&db).unwrap();
    assert_eq!(cookies.len(), 2);
    let auth = cookies.iter().find(|cookie| cookie["name"] == "auth-token").unwrap();
    assert_eq!(auth["domain"], "twitch.tv");
    assert_eq!(auth["hostOnly"], false);
    assert_eq!(auth["httpOnly"], false);
    assert_eq!(auth["secure"], true);
    let local = cookies.iter().find(|cookie| cookie["name"] == "local").unwrap();
    assert_eq!(local["hostOnly"], true);
    assert_eq!(local["httpOnly"], true);
    assert_eq!(local["path"], "/directory");
    assert!(local["expires"].is_null());
    db.execute_batch("PRAGMA user_version = 16; UPDATE moz_cookies SET expiry = expiry * 1000;").unwrap();
    assert_eq!(twitch_cookies(&db).unwrap(), cookies);
}
#[cfg(test)]
#[test]
fn imports_only_youtube_and_preserves_cookie_scope() {
    let cookies = youtube_cookies(concat!(
        "#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t123\tSAPISID\ttest\n",
        "www.youtube.com\tFALSE\t/watch\tFALSE\t0\tLOCAL\ttest\n",
        ".other.example\tTRUE\t/\tTRUE\t123\tSECRET\tignored\n",
    ));
    assert_eq!(cookies.len(), 2);
    assert_eq!(cookies[0]["httpOnly"], true);
    assert_eq!(cookies[0]["hostOnly"], false);
    assert_eq!(cookies[1]["hostOnly"], true);
    assert!(cookies[1]["expires"].is_null());
}

#[test]
fn home_pages_keep_their_snapshot_and_off_disables_long_reads() {
    fn feed(label: &str) -> Recommendations {
        Recommendations {
            seed_video_id: None,
            signed_in: true,
            videos: (0..8).map(|index| Video {
                title: Some(format!("{label} {index}")), url: format!("https://example.com/{label}/{index}"),
                uploader: None, published: None, avatar_url: None, duration_seconds: None,
            }).collect(),
        }
    }
    let mut home = home_cache().0.lock().unwrap();
    home.interval_minutes = 30;
    publish_home(&mut home, feed("old"));
    let revision = home.revision;
    drop(home);
    let first = home_page(None, 0, SHORT_PAGE_SIZE).unwrap();
    assert_eq!(first["videos"].as_array().unwrap().len(), 5);
    assert_eq!(first["next_offset"], 5);
    publish_home(&mut home_cache().0.lock().unwrap(), feed("new"));
    let next = home_page(Some(revision), 5, SHORT_PAGE_SIZE).unwrap();
    assert_eq!(next["videos"][0]["title"], "old 5");
    assert!(next["next_offset"].is_null());
    assert_eq!(home_page(None, 0, LONG_LIMIT).unwrap()["videos"][0]["title"], "new 0");
    set_home_interval(0);
    assert_eq!(home_page(None, 0, LONG_LIMIT).unwrap_err().0, 409);
    assert!(refresh_home().is_err());
    invalidate_home();
}
