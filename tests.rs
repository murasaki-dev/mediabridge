use super::*;

fn item(url: &str, generated: bool) -> QueueItem {
    let mut item: QueueItem = serde_json::from_value(serde_json::json!({"url": url})).unwrap();
    item.autoplay_generated = generated;
    item
}

fn post(state: &mut State, path: &str, body: &'static str) {
    let cfg = Config {
        bind: "127.0.0.1:8765".into(), moonlight: "moonlight".into(), moonlight_args: vec![],
        ytdlp_cookies_from_browser: None, cookies_browser: "off".into(), cookies_profile: None,
        remote_origin_servers: vec![], default_screen: None, default_quality: Some("720".into()),
        default_video_codec: "h264".into(), subtitles: false, preferred_language: None,
    };
    let request = tiny_http::TestRequest::new().with_method(Method::Post).with_path(path).with_body(body);
    handle(request.into(), &cfg, state);
}

#[test]
fn batch_routes_preserve_order_and_validate_before_mutation() {
    let mut state = State::default();
    state.current = Some(item("https://example.com/current", false));
    state.queue.push_back(item("https://example.com/auto", true));
    post(&mut state, "/queue_batch", r#"{"mode":"next","items":[{"url":"https://example.com/a","autoplay":true},{"url":"https://example.com/b"}]}"#);
    assert_eq!(state.queue.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(), ["https://example.com/a", "https://example.com/b", "https://example.com/auto"]);
    assert!(!state.autoplay_enabled);
    post(&mut state, "/queue_batch", r#"{"mode":"last","items":[{"url":"https://example.com/c"},{"url":"https://example.com/d"}]}"#);
    assert_eq!(state.queue.back().unwrap().url, "https://example.com/d");
    assert!(state.queue[2].autoplay_generated);
    let before = state.queue.iter().map(|i| i.url.clone()).collect::<Vec<_>>();
    for invalid in [
        r#"{"mode":"last","items":[{"url":"https://example.com/valid"},{"url":"smb://user:password@host/share/file","local":true}]}"#,
        r#"{"mode":"next","items":[{"url":"https://example.com/valid"},{"url":"https://example.com/invalid-screen","screen":33}]}"#,
        r#"{"mode":"invalid","items":[{"url":"https://example.com/a"}]}"#,
        r#"{"mode":"last","items":[]}"#,
    ] {
        post(&mut state, "/queue_batch", invalid);
        assert_eq!(state.queue.iter().map(|i| i.url.clone()).collect::<Vec<_>>(), before);
        assert_eq!(state.current.as_ref().unwrap().url, "https://example.com/current");
    }
}

#[test]
fn queue_tail_promotion_and_error_dismissal_routes() {
    let mut state = State::default();
    state.current = Some(item("https://example.com/current", false));
    state.queue.push_back(item("https://example.com/auto", true));
    post(&mut state, "/queue", r#"{"url":"https://example.com/tail","at_end":true}"#);
    assert_eq!(state.queue.back().unwrap().url, "https://example.com/tail");
    post(&mut state, "/queue/promote", r#"{"index":0,"queued_by_user_id":"promoter"}"#);
    assert!(!state.queue[0].autoplay_generated);
    assert_eq!(state.queue[0].queued_by_user_id.as_deref(), Some("promoter"));
    let retry = std::time::Instant::now() + Duration::from_secs(60);
    state.autoplay_retry_at = Some(retry);
    state.autoplay_error = Some("diagnostic".into());
    post(&mut state, "/autoplay/error/clear", "{}");
    assert!(state.autoplay_error.is_none());
    assert_eq!(state.autoplay_retry_at, Some(retry));
    assert_eq!(state.queue.len(), 2);
}

#[test]
fn manual_additions_preserve_mix_order() {
    let mut state = State::default();
    state.queue.extend([item("https://example.com/a", false), item("https://example.com/x", true), item("https://example.com/y", true)]);
    enqueue_explicit_item(&mut state, item("https://example.com/b", false));
    assert_eq!(state.queue.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(), ["https://example.com/a", "https://example.com/b", "https://example.com/x", "https://example.com/y"]);
}

#[test]
fn stale_autoplay_results_cannot_fill_queue() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut state = State::default();
    state.autoplay_enabled = true;
    state.autoplay_generation = 2;
    state.current = Some(item("https://www.youtube.com/watch?v=abcdefghijk", false));
    state.autoplay_rx = Some(rx);
    tx.send((1, state.current.as_ref().unwrap().url.clone(), Ok(vec!["https://www.youtube.com/watch?v=lmnopqrstuv".into()]))).unwrap();
    drain_autoplay_results(&mut state);
    assert!(state.queue.is_empty());
}

#[test]
fn disabling_autoplay_releases_unplayed_suggestions() {
    let mut state = State::default();
    let suggested = "https://www.youtube.com/watch?v=abcdefghijk";
    state.queue.extend([item("https://example.com/manual", false), item(suggested, true)]);
    state.autoplay_seen_ids.insert("abcdefghijk".into());
    let cancel = Arc::new(AtomicBool::new(false));
    state.autoplay_cancel = Some(cancel.clone());
    set_autoplay(&mut state, false, None);
    assert_eq!(state.queue.len(), 1);
    assert!(!state.autoplay_seen_ids.contains("abcdefghijk"));
    assert!(cancel.load(Ordering::Acquire));
    assert_eq!(state.autoplay_generation, 1);
}

#[test]
fn codec_fallbacks_all_obey_height_cap() {
    for codec in ["h264", "vp9", "auto"] {
        let selector = ytdl_format_selector(codec, Some("720")).unwrap();
        assert!(selector.split('/').all(|fallback| fallback.contains("[height<=720]")));
        assert!(!ytdl_format_selector(codec, Some("best")).unwrap().contains("height"));
    }
    assert!(ytdl_format_selector("av1", None).is_err());
    assert!(ytdl_format_selector("auto", Some("9000")).is_err());
}

#[test]
fn smb_rejects_credentials_and_playlist_injection() {
    for url in ["smb://user:secret@host/share/file.mp4", "smb://host/share/file\nhttp://evil/file", "smb://host/share/file\rname"] {
        assert!(clean_media_url(url, true).is_err());
    }
    assert!(clean_media_url("smb://host/share/file.mp4", false).is_err());
    let metadata = resolve_media_url("smb://host/share/movie.mp4").unwrap();
    assert_eq!(metadata["title"], "movie.mp4");
}

#[test]
fn private_smb_urls_are_encoded_and_relays_strictly_bound() {
    let credentials = serde_json::json!({"username":"a@b", "password":"secret\n:@", "domain":"PC"});
    assert_eq!(smb_private_playback_url("smb://host/share/file", &credentials, 8081).unwrap(), "smb://PC;a%40b:secret%0A%3A%40@host/share/file");
    let capability = "a".repeat(64);
    let valid = format!("http://127.0.0.1:8081/api/internal/smb-media/{capability}");
    assert_eq!(smb_private_playback_url("smb://host/share/file", &serde_json::json!({"playback_url":valid}), 8081).unwrap(), valid);
    for invalid in [valid.replace(":8081", ":8082"), valid.replace("127.0.0.1", "example.com"), format!("{valid}?extra=1"), format!("{valid}\n")] {
        assert!(smb_private_playback_url("smb://host/share/file", &serde_json::json!({"playback_url":invalid}), 8081).is_err());
    }
}

#[test]
fn public_history_excludes_smb_reference_and_private_url() {
    let mut state = State::default();
    let mut entry = item("smb://host/share/file", false);
    entry.smb_share_id = Some("private-reference".into());
    entry.queued_by_user_id = Some("caller-id".into());
    state.history.push_back(entry);
    state.history.push_back(item("https://example.com/newest", false));
    let value = serde_json::to_value(history_status(&state)).unwrap();
    assert_eq!(value[0]["url"], "https://example.com/newest");
    assert_eq!(value[1]["queued_by_user_id"], "caller-id");
    assert!(!value.to_string().contains("private-reference"));
}

#[test]
fn smb_diagnostics_do_not_echo_secrets_or_renderer_failures() {
    assert!(smb_playback_error_from_line("[vo/gpu] failed to open driver: no such file").is_none());
    let message = smb_playback_error_from_line("[ffmpeg] smb://user:secret@host/share/file: authentication failed").unwrap();
    assert!(!message.contains("secret"));
    assert!(message.contains("sign-in"));
}
