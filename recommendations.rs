use super::*;
use std::sync::Mutex;

static PROFILE_LOCK: Mutex<()> = Mutex::new(());

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
    #[cfg(windows)]
    let root = PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
        .join("MediaBridge");
    #[cfg(not(windows))]
    let root = user_config_path()?
        .parent()
        .ok_or("missing configuration directory")?
        .to_owned();
    let profile = root.join("obscura-profile");
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
    let staging = dir.join("cookies.import");
    let mut file = std::fs::File::create(&staging)
        .map_err(|e| format!("could not stage Obscura cookies: {e}"))?;
    let result = serde_json::to_writer(&mut file, &cookies)
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

#[derive(Serialize, Deserialize)]
pub(super) struct Video {
    pub(super) title: Option<String>,
    pub(super) url: String,
}

#[derive(Serialize)]
pub(super) struct Recommendations {
    pub(super) seed_video_id: String,
    pub(super) signed_in: bool,
    pub(super) videos: Vec<Video>,
}

const EXTRACT: &str = r#"(() => {
    const root = globalThis.ytInitialData?.contents?.twoColumnWatchNextResults?.secondaryResults;
    const seen = new Set();
    const videos = [];
    function visit(node) {
        if (!node || typeof node !== 'object') return;
        const card = node.lockupViewModel || node.compactVideoRenderer || node.videoRenderer;
        if (card) {
            const endpoint = (card.navigationEndpoint || card.rendererContext?.commandContext?.onTap?.innertubeCommand)?.watchEndpoint;
            const id = endpoint?.videoId;
            if (/^[A-Za-z0-9_-]{11}$/.test(id || '') && !endpoint.playlistId && !seen.has(id)) {
                const title = card.metadata?.lockupMetadataViewModel?.title?.content
                    || card.title?.simpleText || card.title?.runs?.map(run => run.text).join('');
                seen.add(id);
                videos.push({id, title: title || null});
            }
            return;
        }
        for (const child of Object.values(node)) visit(child);
    }
    visit(root);
    return {signed_in: globalThis.ytcfg?.get?.('LOGGED_IN') === true, available: !!root, videos};
})()"#;

pub(super) fn lookup(seed: String, cancel: &AtomicBool) -> Result<Recommendations, String> {
    let _guard = PROFILE_LOCK
        .try_lock()
        .map_err(|_| "a recommendation lookup is already running")?;
    let mut command = media_tool_command("obscura");
    #[cfg(target_os = "linux")]
    if let Some(path) = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("obscura")))
        .filter(|path| path.is_file())
    {
        command = Command::new(path);
    }
    command.env("RUST_LOG", "off");
    command
        .arg("fetch")
        .arg("--storage-dir")
        .arg(profile()?)
        .args([
            &format!("https://www.youtube.com/watch?v={seed}"),
            "--quiet",
            "--timeout",
            "15",
            "--wait-until",
            "domcontentloaded",
            "--wait",
            "0",
            "--eval",
            EXTRACT,
        ]);
    let output = run_cancellable_command(command, cancel).map_err(|e| format!("Obscura: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Obscura could not load YouTube ({})",
            output.status
        ));
    }
    #[derive(Deserialize)]
    struct Candidate {
        id: String,
        title: Option<String>,
    }
    #[derive(Deserialize)]
    struct WatchData {
        signed_in: bool,
        available: bool,
        videos: Vec<Candidate>,
    }
    let data: WatchData = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Obscura returned invalid recommendation data")?;
    if !data.available {
        return Err("YouTube did not provide watch-page recommendations".into());
    }
    let videos = data
        .videos
        .into_iter()
        .filter(|video| video.id != seed && valid_id(&video.id))
        .take(20)
        .map(|video| Video {
            title: video.title,
            url: format!("https://www.youtube.com/watch?v={}", video.id),
        })
        .collect();
    Ok(Recommendations {
        seed_video_id: seed,
        signed_in: data.signed_in,
        videos,
    })
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
