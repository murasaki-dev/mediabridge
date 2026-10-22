use super::*;

#[test]
fn flatpak_host_access_uses_effective_session_policy() {
    assert!(!flatpak_host_access_from_info("[Application]\norg.freedesktop.Flatpak=talk\n"));
    assert!(!flatpak_host_access_from_info("[Session Bus Policy]\norg.freedesktop.Notifications=talk\n"));
    assert!(flatpak_host_access_from_info("[Session Bus Policy]\norg.freedesktop.Flatpak=talk\n"));
    assert!(flatpak_host_access_from_info("[Session Bus Policy]\norg.freedesktop.*=talk\n"));
    assert!(!flatpak_host_access_from_info("[Session Bus Policy]\norg.freedesktop.*=talk\norg.freedesktop.Flatpak=none\n"));
    assert!(!flatpak_host_access_from_info("[Session Bus Policy]\norg.freedesktop.Flatpak=none\norg.freedesktop.*=talk\n"));
}

#[test]
fn mpv_feedback_is_transient_and_track_labels_are_literal_and_bounded() {
    let options = mpv_osd_options();
    assert!(options.contains(&"--osd-level=1".into()));
    assert!(options.contains(&"--osd-duration=1200".into()));
    assert!(options.contains(&"--osd-playing-msg-duration=2000".into()));
    assert!(options.contains(&"--osd-on-seek=msg-bar".into()));
    assert_eq!(track_feedback(true,None),"Subtitles: Off");
    assert_eq!(track_feedback(false,None),"Audio: Off");
    let tracks=media_tracks(&serde_json::json!([{"id":4,"type":"audio","lang":"eng","title":"Original"}]),"audio");
    assert_eq!(track_feedback(false,tracks.first()),"Audio: eng · Original");
    assert_eq!(brief_osd_text("${path}\n\u{1b}text"),"${path}text");
    assert_eq!(brief_osd_text(&"a".repeat(1000)).len(),180);
}

#[test]
fn track_lists_are_typed_and_do_not_expose_private_source_paths() {
    let raw = serde_json::json!([
        {"id":1,"type":"audio","lang":"eng","selected":true,"demux-channel-count":2},
        {"id":2,"type":"sub","lang":"spa","title":"Spanish","external":true,"external-filename":"http://private/secret"},
        {"id":3,"type":"video"}
    ]);
    let audio = media_tracks(&raw,"audio"); let subtitles = media_tracks(&raw,"sub");
    assert_eq!(audio.len(),1); assert!(audio[0].selected); assert_eq!(audio[0].channels,Some(2));
    assert_eq!(subtitles.len(),1); assert!(subtitles[0].external);
    assert!(!serde_json::to_string(&subtitles).unwrap().contains("secret"));
}

#[test]
fn preferred_tracks_follow_language_and_title_instead_of_numeric_ids() {
    let raw = serde_json::json!([
        {"id":7,"type":"audio","lang":"eng","title":"Commentary"},
        {"id":12,"type":"audio","lang":"eng","title":"Original"},
        {"id":2,"type":"audio","lang":"spa","title":"Original"}
    ]);
    let tracks = media_tracks(&raw,"audio");
    let choice = TrackMatch {language:Some("ENG".into()),title:Some("Original".into())};
    assert_eq!(matching_track(&tracks,&choice).unwrap().id,12);
    let unavailable = TrackMatch {language:Some("jpn".into()),title:Some("Original".into())};
    assert!(matching_track(&tracks,&unavailable).is_none());
    let settings = serde_json::from_str::<UserConfig>("{}").unwrap();
    assert!(!settings.track_preferences.load_multiple_audio_tracks);
    let selector = ytdl_format_selector("h264",Some("720")).unwrap();
    assert!(audio_format_selector(&selector,false).contains("ba[format_note*=original]"));
    assert_eq!(audio_format_selector(&selector,true),selector);
}

#[test]
fn track_routes_reject_stale_playback_and_non_mpv_without_mutation() {
    let mut state = State::default(); state.playback_epoch=42;
    post(&mut state,"/subtitles/select",r#"{"id":1,"remember":false,"playback_epoch":41}"#);
    post(&mut state,"/audio/select",r#"{"id":1,"remember":false}"#);
    assert_eq!(state.playback_epoch,42); assert!(state.current.is_none());
}

#[test]
#[ignore = "requires the isolated mpv fixture launched by tests/mpv-tracks.ps1"]
fn real_mpv_track_switches_and_off_preserve_loaded_lists() {
    let pipe = std::env::var_os("MEDIABRIDGE_TEST_TRACK_PIPE").expect("run tests/mpv-tracks.ps1");
    let socket = PathBuf::from(pipe);
    let raw = mpv_property(&socket,"track-list").expect("mpv track-list");
    let subtitles=media_tracks(&raw,"sub"); let audio=media_tracks(&raw,"audio");
    assert!(subtitles.len()>=2 && audio.len()>=2,"fixture must expose two tracks of each kind");
    let mut state=State::default(); let mut current=item("https://example.com/fixture",false); current.local_playback=true;
    state.current=Some(current); state.mpv_socket=Some(socket.clone()); state.playback_epoch=9;
    // Verify mpv accepts the literal-text and OSD seek prefixes over real IPC.
    mpv_ipc(&socket,serde_json::json!(["raw","show-text","Literal ${path}",1200])).unwrap();
    mpv_ipc(&socket,serde_json::json!(["osd-msg-bar","seek",1,"absolute"])).unwrap();
    control_mpv(&mut state,"resume",None).unwrap();
    assert_eq!(mpv_property(&socket,"pause"),Some(serde_json::json!(false)));
    control_mpv(&mut state,"pause",None).unwrap();
    assert_eq!(mpv_property(&socket,"pause"),Some(serde_json::json!(true)));
    control_mpv(&mut state,"play_pause",None).unwrap();
    control_mpv(&mut state,"pause",None).unwrap();
    control_mpv(&mut state,"seek",Some(2.0)).unwrap();
    control_mpv(&mut state,"forward",None).unwrap();
    control_mpv(&mut state,"rewind",None).unwrap();
    control_mpv(&mut state,"volume",Some(0.3)).unwrap();
    // ao=null can omit the volume property; real outputs retain it normally.
    if mpv_property(&socket,"volume").is_some() { assert_eq!(state.last_mpv_volume,Some(0.3)); }
    for (path,property,tracks) in [("/subtitles/select","sid",subtitles),("/audio/select","aid",audio)] {
        for track in &tracks {
            let body = Box::leak(format!("{{\"id\":{},\"remember\":false,\"playback_epoch\":9}}",track.id).into_boxed_str());
            post(&mut state,path,body);
            assert_eq!(mpv_property(&socket,property).and_then(|v|v.as_i64()),Some(track.id));
        }
        post(&mut state,path,r#"{"id":null,"remember":false,"playback_epoch":9}"#);
        let disabled = mpv_property(&socket,property).unwrap();
        assert!(disabled == serde_json::json!("no") || disabled == serde_json::json!(false), "unexpected disabled track value: {disabled}");
        let raw=mpv_property(&socket,"track-list").unwrap();
        assert_eq!(media_tracks(&raw,if property=="sid" {"sub"} else {"audio"}).len(),tracks.len());
    }
}

#[test]
fn close_countdown_accepts_custom_minutes_and_is_independent_of_autoplay() {
    let mut state = State::default();
    set_close_timer(&mut state, Some(2880.5)).unwrap();
    let original = state.close_deadline;
    set_autoplay(&mut state, false, None);
    assert_eq!(state.close_deadline, original);
    set_close_timer(&mut state, Some(0.25)).unwrap();
    let remaining = state.close_deadline.unwrap().duration_since(std::time::Instant::now()).as_secs_f64();
    assert!(remaining > 14.0 && remaining <= 15.0);
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        let saved = state.close_deadline;
        assert!(set_close_timer(&mut state, Some(invalid)).is_err());
        assert_eq!(state.close_deadline, saved);
    }
    set_close_timer(&mut state, None).unwrap();
    assert!(state.close_deadline.is_none());
}

#[test]
fn countdown_expiry_closes_playback_and_cancels_work_once() {
    let mut state = State::default();
    let mut current = item("https://example.com/current", false); current.local_playback = true;
    state.current = Some(current);
    state.queue.push_back(item("https://example.com/queued", false));
    state.history.push_back(item("https://example.com/history", false));
    let metadata_cancel = Arc::new(AtomicBool::new(false));
    let autoplay_cancel = Arc::new(AtomicBool::new(false));
    state.metadata_cancel = Some(metadata_cancel.clone()); state.autoplay_cancel = Some(autoplay_cancel.clone());
    state.autoplay_enabled = true;
    let now = std::time::Instant::now(); state.close_deadline = Some(now + Duration::from_secs(1));
    assert!(!expire_close_timer(&mut state, now));
    assert!(state.current.is_some());
    assert!(expire_close_timer(&mut state, now + Duration::from_secs(2)));
    assert!(state.current.is_none() && state.queue.is_empty() && state.close_deadline.is_none());
    assert!(!state.autoplay_enabled);
    assert!(metadata_cancel.load(Ordering::Acquire) && autoplay_cancel.load(Ordering::Acquire));
    assert_eq!(state.history.len(), 1);
    assert!(!expire_close_timer(&mut state, now + Duration::from_secs(3)));
}

#[test]
fn timer_route_replaces_cancels_and_legacy_minutes_schedule_close() {
    let mut state = State::default();
    post(&mut state, "/timer", r#"{"minutes":12.5}"#);
    assert!(state.close_deadline.is_some());
    let saved = state.close_deadline;
    post(&mut state, "/timer", r#"{"minutes":-1}"#);
    assert_eq!(state.close_deadline, saved);
    post(&mut state, "/timer", r#"{"minutes":null}"#);
    assert!(state.close_deadline.is_none());
    post(&mut state, "/autoplay", r#"{"enabled":false,"duration_minutes":2.5}"#);
    assert!(state.close_deadline.is_some());
    assert!(!state.autoplay_enabled);
}

#[test]
fn interruption_keeps_position_settings_and_manual_queue_priority() {
    let mut current = item("smb://host/share/video.mp4", true);
    current.local_playback = true;
    current.smb_share_id = Some("0123456789abcdef01234567".into());
    current.quality = Some("1080".into());
    let mut status = media_status(&mut State::default());
    status.can_seek = true;
    status.position_seconds = Some(125.75);
    let interrupted = interrupted_item(&current, &status).unwrap();
    assert_eq!(interrupted.start_position_seconds, Some(125.75));
    assert_eq!(interrupted.smb_share_id, current.smb_share_id);
    assert_eq!(interrupted.quality, current.quality);
    assert!(!interrupted.autoplay_generated);
    let mut state = State::default();
    state.queue.push_back(item("https://example.com/waiting", false));
    state.queue.push_front(interrupted);
    set_autoplay(&mut state, false, None);
    assert_eq!(state.queue[0].start_position_seconds, Some(125.75));
    assert_eq!(state.queue[1].url, "https://example.com/waiting");
    assert_eq!(serde_json::to_value(queue_status(&state)).unwrap()[0]["start_position_seconds"], 125.75);
    for position in [None, Some(-1.0), Some(f64::NAN)] {
        status.position_seconds = position;
        assert!(interrupted_item(&current, &status).is_err());
    }
    status.position_seconds = Some(1.0); status.can_seek = false;
    assert!(interrupted_item(&current, &status).is_err());
}

#[test]
fn mpv_resume_positions_override_watch_later_for_smb_and_web_media() {
    for url in ["smb://host/share/movie.mp4", "https://example.com/movie.mp4"] {
        let args = mpv_start_options(url, Some(125.75));
        assert!(args.contains(&"--start=125.75".into()));
        assert!(args.contains(&"--resume-playback=no".into()));
        assert!(!args.contains(&"--start=0".into()));
    }
    assert!(mpv_start_options("smb://host/share/movie.mp4", None).contains(&"--start=0".into()));
    assert!(mpv_start_options("https://example.com/movie.mp4", Some(0.0)).contains(&"--start=0".into()));
}

#[test]
fn invalid_resume_values_do_not_change_queue_or_current_item() {
    let mut state = State::default();
    state.current = Some(item("https://example.com/current", false));
    for path in ["/play_interrupt", "/queue", "/play_preserve_queue"] {
        post(&mut state, path, r#"{"url":"https://example.com/new","start_position_seconds":-1}"#);
        assert!(state.queue.is_empty());
        assert_eq!(state.current.as_ref().unwrap().url, "https://example.com/current");
    }
    post(&mut state, "/queue_batch", r#"{"mode":"next","items":[{"url":"https://example.com/new","start_position_seconds":-1}]}"#);
    assert!(state.queue.is_empty());
}

#[test]
fn interruption_without_observable_player_position_preserves_current_and_queue() {
    let mut state = State::default();
    let mut current = item("https://example.com/current", false);
    current.local_playback = true;
    state.current = Some(current);
    state.queue.push_back(item("https://example.com/waiting", false));
    post(&mut state, "/play_interrupt", r#"{"url":"https://example.com/new","local":true}"#);
    assert_eq!(state.current.as_ref().unwrap().url, "https://example.com/current");
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0].url, "https://example.com/waiting");
    assert!(state.history.is_empty());
}

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
        default_video_codec: "h264".into(), subtitles: false, preferred_language: None, track_preferences: Default::default(),
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
