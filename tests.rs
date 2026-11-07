use super::*;

#[test]
fn api_smb_credentials_are_private_and_survive_resume() {
    let entry: QueueItem = serde_json::from_value(serde_json::json!({
        "url":"smb://host/share/movie.mp4", "local":true,
        "smb_credentials":{"username":"alice", "password":"private:@secret", "domain":"PC"}
    })).unwrap();
    assert!(validate_smb_credentials(&entry).is_ok());
    assert_eq!(smb_authenticated_playback_url(&entry).unwrap().unwrap(),
        "smb://PC;alice:private%3A%40secret@host/share/movie.mp4");
    let public = serde_json::to_string(&status_items(std::iter::once(&entry))).unwrap();
    assert!(!public.contains("private"));
    assert!(!public.contains("credentials"));
    let mut status = media_status(&mut State::default());
    status.can_seek = true;
    status.position_seconds = Some(12.0);
    let resumed = interrupted_item(&entry, &status).unwrap();
    assert_eq!(resumed.smb_credentials, entry.smb_credentials);
    assert_eq!(resumed.start_position_seconds, Some(12.0));
    let mut invalid = entry.clone();
    invalid.url = "https://example.com/video".into();
    assert!(validate_smb_credentials(&invalid).is_err());
    invalid = entry.clone(); invalid.local_playback = false;
    assert!(validate_smb_credentials(&invalid).is_err());
    invalid = entry; invalid.smb_share_id = Some("0123456789abcdef01234567".into());
    assert!(validate_smb_credentials(&invalid).is_err());
}

#[test]
fn invalid_smb_credentials_do_not_echo_secrets() {
    for credentials in [serde_json::json!({"username":"user","password":["private-secret"]}),
        serde_json::json!({"username":"","password":"private-secret"}),
        serde_json::json!({"username":"user","password":"private-secret","playback_url":"http://evil"})] {
        let result = serde_json::from_value::<QueueItem>(serde_json::json!({"url":"smb://host/share/file", "local":true,"smb_credentials":credentials}));
        let error = result.err().expect("invalid credentials must fail").to_string();
        assert!(!error.contains("private-secret"));
    }
}

#[test]
fn explicit_receiver_stop_clears_playback_queue_and_autoplay() {
    let mut state=State::default();
    state.current=Some(receiver_item("abcdefghijk",0.0));
    state.queue.push_back(receiver_item("lmnopqrstuv",0.0));
    state.youtube_receiver_playlist=vec!["abcdefghijk".into(),"lmnopqrstuv".into()];
    state.autoplay_enabled=true;
    state.repeat_playlist_enabled=true;
    apply_receiver_command(&mut state,&test_config(),"remoteDisconnected",&serde_json::json!({"id":"a"})).unwrap();
    assert!(state.current.is_some());assert_eq!(state.queue.len(),1);
    apply_receiver_command(&mut state,&test_config(),"stopVideo",&serde_json::json!({})).unwrap();
    assert!(state.current.is_none());assert!(state.queue.is_empty());assert!(!state.autoplay_enabled);
    assert!(!state.repeat_playlist_enabled);assert!(state.youtube_receiver_playlist.is_empty());
}

#[test]
fn opening_mpv_defers_all_metadata_and_requeues_preempted_lookup() {
    let mut state=State::default();
    let mut current=item("https://www.youtube.com/watch?v=abcdefghijk",false);
    current.local_playback=true;
    state.current=Some(current);
    state.mpv_socket=Some(std::env::temp_dir().join(format!("mediabridge-missing-pipe-{}",std::process::id())));
    state.pending_metadata.push_back("https://www.youtube.com/watch?v=abcdefghijk".into());
    state.pending_queue_metadata.push_back("https://www.youtube.com/watch?v=lmnopqrstuv".into());
    let (tx,_rx)=mpsc::channel();state.metadata_tx=Some(tx);
    dispatch_pending_metadata(&mut state);
    assert!(!state.metadata_in_flight);
    assert_eq!(state.pending_metadata.len(),1);
    assert_eq!(state.pending_queue_metadata.len(),1);
    let (tx,rx)=mpsc::channel();state.metadata_rx=Some(rx);
    state.metadata_in_flight=true;
    state.metadata_in_flight_url=Some("https://www.youtube.com/watch?v=01234567890".into());
    state.metadata_cancel=Some(Arc::new(AtomicBool::new(true)));
    tx.send(("https://www.youtube.com/watch?v=01234567890".into(),Err("background media lookup was cancelled".into()))).unwrap();
    drain_metadata(&mut state);
    assert!(!state.metadata_in_flight);
    assert_eq!(state.pending_queue_metadata.len(),2);
    assert!(state.current.as_ref().unwrap().metadata_error.is_none());
}

#[test]
fn receiver_playlist_validation_queue_updates_and_idle_volume() {
    let (ids,index,time)=receiver_playlist(&serde_json::json!({"videoIds":"abcdefghijk,lmnopqrstuv","currentIndex":"1","currentTime":"12.5"})).unwrap();
    assert_eq!(ids.len(),2);assert_eq!(index,1);assert_eq!(time,12.5);
    for args in [serde_json::json!({"videoIds":"https://attacker.test"}),serde_json::json!({"videoIds":"abcdefghijk","currentIndex":"2"}),
        serde_json::json!({"videoId":"abcdefghijk","currentTime":"NaN"}),serde_json::json!({"videoId":"abcdefghijk","currentTime":"-1"})] {
        assert!(receiver_playlist(&args).is_err());
    }
    let mut state=State::default();
    apply_receiver_command(&mut state,&test_config(),"setVolume",&serde_json::json!({"volume":"37"})).unwrap();
    assert_eq!(state.last_mpv_volume,Some(0.37));
    state.current=Some(receiver_item("abcdefghijk",0.0));
    apply_receiver_command(&mut state,&test_config(),"updatePlaylist",&serde_json::json!({"videoIds":"lmnopqrstuv,abcdefghijk,01234567890","listId":"TEST_LIST"})).unwrap();
    assert_eq!(state.queue.len(),1);
    assert!(state.queue[0].local_playback);
    assert_eq!(youtube_video_id(&state.queue[0].url).as_deref(),Some("01234567890"));
    assert!(apply_receiver_command(&mut state,&test_config(),"updatePlaylist",&serde_json::json!({"videoIds":"lmnopqrstuv"})).is_err());
    assert_eq!(state.queue.len(),1);
}

#[test]
fn dial_http_discovery_settings_and_redacted_launch_diagnostics() {
    let directory=std::env::temp_dir().join(format!("mediabridge-dial-test-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let path=directory.join("dial.json");
    let server=Server::http("127.0.0.1:0").unwrap();
    let address=server.server_addr().to_ip().unwrap();
    let test_path=path.clone();
    let worker=std::thread::spawn(move|| {
        let mut cfg=test_config();cfg.bind=address.to_string();
        let mut state=State::default();
        state.dial=Some(dial::Discovery::start(test_path,address).unwrap());
        for _ in 0..17 {
            let request=server.recv_timeout(Duration::from_secs(5)).unwrap().expect("test HTTP request");
            handle(request,&cfg,&mut state);
        }
    });
    let call=|method:&str,path:&str,body:&str,origin:Option<&str>| {
        let mut stream=TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",body.len()).unwrap();
        if let Some(origin)=origin {write!(stream,"Origin: {origin}\r\n").unwrap();}
        write!(stream,"\r\n{body}").unwrap();
        let mut response=String::new();stream.read_to_string(&mut response).unwrap();response
    };
    let device=call("GET","/dial/device.xml","",None);
    assert!(device.starts_with("HTTP/1.1 200"));
    assert!(device.to_lowercase().contains(&format!("application-url: http://{address}/dial/apps/")));
    let android_device=call("GET","/dial/device.xml","",Some("package:com.google.android.youtube"));
    assert!(android_device.starts_with("HTTP/1.1 200"));
    assert!(android_device.to_lowercase().contains("access-control-allow-origin: package:com.google.android.youtube"));
    assert!(android_device.to_lowercase().contains(&format!("application-url: http://{address}/dial/apps/")));
    assert!(call("GET","/dial/apps/YouTube","",Some("package:com.google.android.youtube")).starts_with("HTTP/1.1 200"));
    assert!(call("GET","/dial/apps/Netflix","",Some("package:com.google.android.youtube")).starts_with("HTTP/1.1 403"));
    assert!(call("GET","/dial/device.xml","",Some("package:com.google.android.youtube.attacker")).starts_with("HTTP/1.1 403"));
    let chrome=call("GET","/dial/apps/YouTube","",Some("package:Google-Chrome.150.Windows"));
    assert!(chrome.starts_with("HTTP/1.1 200"));
    assert!(chrome.to_lowercase().contains("access-control-allow-origin: package:google-chrome.150.windows"));
    assert!(call("GET","/dial/device.xml","",Some("package:Chromium.150.Linux")).starts_with("HTTP/1.1 200"));
    assert!(call("GET","/dial/device.xml","",Some("package:Google-Chrome.attacker.Windows")).starts_with("HTTP/1.1 403"));
    assert!(call("GET","/settings/dial","",Some("package:Google-Chrome.150.Windows")).starts_with("HTTP/1.1 403"));
    assert!(call("POST","/dial/apps/YouTube","unknown=fixture",Some("package:Google-Chrome.150.Mac-OS-X")).starts_with("HTTP/1.1 422"));
    assert!(call("GET","/dial/apps/YouTube","",None).contains("<state>stopped</state>"));
    assert!(call("POST","/dial/apps/YouTube","token=PRIVATE_VALUE_NOT_TO_LOG&unknown=x",None).starts_with("HTTP/1.1 422"));
    let status=call("GET","/settings/dial","",None);
    assert!(status.contains("unsupported launch data"));assert!(status.contains("token"));
    assert!(!status.contains("PRIVATE_VALUE_NOT_TO_LOG"));
    let preflight=call("OPTIONS","/dial/apps/YouTube","",Some("https://www.youtube.com"));
    assert!(preflight.starts_with("HTTP/1.1 204"));
    assert!(preflight.to_lowercase().contains("access-control-allow-origin: https://www.youtube.com"));
    assert!(call("OPTIONS","/dial/apps/YouTube","",Some("https://attacker.test")).starts_with("HTTP/1.1 403"));
    assert!(call("POST","/settings/dial",r#"{"enabled":false,"friendly_name":"Living Room"}"#,None).starts_with("HTTP/1.1 200"));
    assert!(call("GET","/dial/device.xml","",None).starts_with("HTTP/1.1 503"));
    worker.join().unwrap();
    let settings:serde_json::Value=serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(settings["enabled"],false);assert_eq!(settings["friendly_name"],"Living Room");
    std::fs::remove_file(path).unwrap();std::fs::remove_dir(directory).unwrap();
}

#[test]
fn playlist_repeat_rotates_in_order_and_does_not_duplicate_on_failed_transition() {
    let mut state=State::default();
    state.current=Some(item("https://example.com/a",false));
    state.queue.extend([item("https://example.com/b",false),item("https://example.com/c",false)]);
    post(&mut state,"/repeat_playlist",r#"{"enabled":true}"#);
    assert!(state.repeat_playlist_enabled);
    for expected in ["b","c","a","b"] {
        requeue_completed_playlist_item(&mut state);
        requeue_completed_playlist_item(&mut state);
        assert_eq!(state.queue.len(),3);
        assert!(!start_front_queued_with(&mut state,|state,_|{state.playback_epoch+=1;state.repeat_playlist_requeued_epoch=None;Err("fixture".into())}));
        requeue_completed_playlist_item(&mut state);
        assert_eq!(state.queue.len(),3);
        state.queue_start_retry_at=None;
        assert!(start_front_queued_with(&mut state,|state,item|{state.playback_epoch+=1;state.repeat_playlist_requeued_epoch=None;state.current=Some(item);Ok(())}));
        assert_eq!(state.current.as_ref().unwrap().url,format!("https://example.com/{expected}"));
        assert_eq!(state.queue.len(),2);
    }
    post(&mut state,"/repeat_playlist",r#"{"enabled":false}"#);
    requeue_completed_playlist_item(&mut state);
    assert_eq!(state.queue.len(),2);
    post(&mut state,"/close","{}");
    assert!(!state.repeat_playlist_enabled);
}

#[test]
fn dial_pairing_uses_receiver_and_codes_are_redacted() {
    assert_eq!(dial::pairing_code("pairingCode=SECRET_CODE").unwrap().as_deref(),Some("SECRET_CODE"));
    assert!(dial::launch("YouTube","pairingCode=SECRET_CODE").is_err());
    assert!(!safe_media_url("https://www.youtube.com/tv?pairingCode=SECRET_CODE").contains("SECRET_CODE"));
    assert!(dial_origin_allowed("https://www.youtube.com",Some("YouTube")));
    assert!(dial_origin_allowed("package:com.google.android.youtube",None));
    for origin in ["package:Google-Chrome.150.Windows","package:Chromium.150.Linux","package:Google-Chrome.150.Mac-OS-X"] {
        assert!(dial_origin_allowed(origin,Some("YouTube")));
    }
    for origin in ["package:Google-Chrome.150.Windows.attacker","package:Google-Chrome.150.Windows/","package:Unknown.150.Windows","package:Google-Chrome..Windows"] {
        assert!(!dial_origin_allowed(origin,None));
    }
    assert!(dial_origin_allowed("package:com.google.android.youtube",Some("YouTube")));
    assert!(!dial_origin_allowed("package:com.google.android.youtube",Some("Twitch")));
    assert!(!dial_origin_allowed("package:com.google.android.youtube/",None));
    assert!(!dial_origin_allowed("package:unknown.app",None));
    assert!(!dial_origin_allowed("https://www.youtube.com.attacker.test",Some("YouTube")));
    assert!(!dial_origin_allowed("https://www.netflix.com",Some("YouTube")));
}

#[test]
fn browser_repeat_restarts_without_consuming_queue_and_surfaces_failures() {
    for fail in [false,true] {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address=listener.local_addr().unwrap();
        let peer=std::thread::spawn(move|| {
            let (stream,_)=listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut socket=tungstenite::accept(stream).unwrap();
            let command:serde_json::Value=serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(command["method"],"script.evaluate");
            assert_eq!(command["params"]["userActivation"],true);
            assert!(command["params"]["expression"].as_str().unwrap().contains("v.currentTime=0"));
            let result=if fail { serde_json::json!({"type":"exception"}) }
                else {serde_json::json!({"type":"success","result":{"type":"string","value":"{\"restarted\":true}"}})};
            socket.send(Message::Text(serde_json::json!({"id":command["id"],"type":"success","result":result}).to_string())).unwrap();
        });
        let (socket,_)=connect(format!("ws://{address}")).unwrap();
        let mut state=State::default();
        let mut current=item("https://example.com/video",false); current.browser=Some("librewolf".into());
        state.current=Some(current); state.queue.push_back(item("https://example.com/next",false));
        state.repeat_enabled=true;
        state.librewolf_bidi=Some(LibreWolfBidi {socket,next_id:0,context_id:Some("tab".into()),
            cached_at:Some(std::time::Instant::now()),cached_video:Some(LibreWolfVideoState{ended:true,..Default::default()}),
            autoplay_attempted:true,fullscreen_attempted_at:None,unmatched_url_reported:None});
        advance(&mut state,&test_config());
        assert_eq!(state.current.as_ref().unwrap().url,"https://example.com/video");
        assert_eq!(state.queue.len(),1);
        assert!(state.history.is_empty());
        assert_eq!(state.repeat_enabled,!fail);
        assert_eq!(state.playback_error.is_some(),fail);
        peer.join().unwrap();
    }
}

#[test]
fn repeat_route_rejects_invalid_requests_and_disables_without_active_playback() {
    let mut state=State::default();
    post(&mut state,"/repeat",r#"{"enabled":true}"#);
    assert!(!state.repeat_enabled);
    state.repeat_enabled=true;
    post(&mut state,"/repeat",r#"{"enabled":"false"}"#);
    assert!(state.repeat_enabled);
    post(&mut state,"/repeat",r#"{"enabled":false,"extra":true}"#);
    assert!(state.repeat_enabled);
    post(&mut state,"/repeat",r#"{"enabled":false}"#);
    assert!(!state.repeat_enabled);
    state.repeat_enabled=true;
    post(&mut state,"/close","{}");
    assert!(!state.repeat_enabled);
}

#[test]
fn browser_pointer_maps_normalized_coordinates_and_clamps_relative_movement() {
    let request=|body|serde_json::from_str::<MouseRequest>(body).unwrap();
    let (actions,point)=browser_mouse_actions(&request(r#"{"action":"click","x":32767,"y":32767}"#),None,800.0,600.0).unwrap();
    assert_eq!(point,(799.0,599.0));
    assert_eq!(actions[0]["actions"][1]["type"],"pointerDown");
    assert_eq!(actions[0]["actions"][2]["type"],"pointerUp");
    let (_,point)=browser_mouse_actions(&request(r#"{"action":"move_relative","dx":100,"dy":-1000}"#),Some(point),800.0,600.0).unwrap();
    assert_eq!(point,(799.0,0.0));
    assert!(browser_mouse_actions(&request(r#"{"action":"click","x":-1}"#),None,800.0,600.0).is_err());
    assert!(browser_mouse_actions(&request(r#"{"action":"move"}"#),None,0.0,600.0).is_err());
}

#[test]
fn managed_browser_requests_activated_youtube_fullscreen_and_leaves_twitch_alone() {
    for theater in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = if theater { "https://www.twitch.tv/channel" } else { "https://www.youtube.com/watch?v=abcdefghijk" };
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            for index in 0..if theater { 2 } else { 3 } {
                let command: serde_json::Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                let result = match index {
                    0 => {
                        assert_eq!(command["method"], "browsingContext.getTree");
                        serde_json::json!({"contexts":[{"context":"tab","url":url}]})
                    }
                    1 => {
                        assert_eq!(command["method"], "script.evaluate");
                        let state = serde_json::json!({"hasVideo":true,"embeddedFullscreen":theater,"paused":false,
                            "theaterActive":true,"chatExpandButton":{"x":25,"y":30}});
                        serde_json::json!({"type":"success","result":{"type":"string","value":state.to_string()}})
                    }
                    2 if !theater => {
                        let expression = command["params"]["expression"].as_str().unwrap();
                        assert!(expression.contains("requestFullscreen"));
                        assert_eq!(command["params"]["userActivation"], true);
                        serde_json::json!({"type":"success","result":{"type":"string","value":"{\"fullscreen\":true}"}})
                    }
                    _ => {
                        assert_eq!(command["method"], "input.performActions");
                        let input = &command["params"]["actions"][0];
                        assert_eq!(input["type"], if theater { "pointer" } else { "key" });
                        if theater { assert_eq!(input["actions"][0]["x"].as_f64(), Some(25.0)); }
                        else { assert_eq!(input["actions"][0]["value"], "f"); }
                        serde_json::json!({})
                    }
                };
                socket.send(Message::Text(serde_json::json!({"id":command["id"],"type":"success","result":result}).to_string())).unwrap();
            }
        });
        let (socket, _) = connect(format!("ws://{address}")).unwrap();
        let mut bidi = LibreWolfBidi {socket,next_id:0,context_id:Some("tab".into()),cached_at:None,cached_video:None,
            autoplay_attempted:true,fullscreen_attempted_at:None,unmatched_url_reported:None};
        assert!(bidi.refresh(url).unwrap().is_some());
        peer.join().unwrap();
    }
}

#[test]
fn queued_launch_failure_preserves_order_and_retries_without_losing_history() {
    let item = |url| serde_json::from_value::<QueueItem>(serde_json::json!({"url":url})).unwrap();
    let mut state = State::default();
    state.current = Some(item("https://example.com/finished"));
    state.queue.push_back(item("https://example.com/next"));
    state.queue.push_back(item("https://example.com/later"));
    assert!(!start_front_queued_with(&mut state, |_state, _item| Err("profile is locked".into())));
    assert_eq!(state.queue.len(), 2);
    assert_eq!(state.queue[0].url, "https://example.com/next");
    assert_eq!(state.current.as_ref().unwrap().url, "https://example.com/finished");
    assert!(state.history.is_empty());
    assert!(state.playback_error.as_ref().unwrap().contains("profile is locked"));
    assert!(!start_front_queued_with(&mut state, |_, _| panic!("retry must be throttled")));
    state.queue_start_retry_at = None;
    assert!(start_front_queued_with(&mut state, |state, item| { state.current = Some(item); Ok(()) }));
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.current.as_ref().unwrap().url, "https://example.com/next");
    assert_eq!(state.history.len(), 1);
    assert!(state.playback_error.is_none());
}

#[test]
fn browser_queue_navigation_reuses_tab_and_preserves_it_on_failure() {
    for fail in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let command: serde_json::Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(command["method"], "browsingContext.navigate");
            assert_eq!(command["params"]["context"], "existing-tab");
            assert_eq!(command["params"]["url"], "https://example.com/next");
            let reply = if fail {
                serde_json::json!({"id":command["id"],"type":"error","message":"navigation failed"})
            } else { serde_json::json!({"id":command["id"],"type":"success","result":{}}) };
            socket.send(Message::Text(reply.to_string())).unwrap();
        });
        let (socket, _) = connect(format!("ws://{address}")).unwrap();
        let mut bidi = LibreWolfBidi {socket, next_id:0, context_id:Some("existing-tab".into()),
            cached_at:Some(std::time::Instant::now()), cached_video:None, autoplay_attempted:true,
            fullscreen_attempted_at:Some(std::time::Instant::now()), unmatched_url_reported:None};
        assert_eq!(bidi.navigate_playback("https://example.com/next").is_err(), fail);
        assert_eq!(bidi.context_id.is_some(), fail);
        assert_eq!(bidi.autoplay_attempted, fail);
        peer.join().unwrap();
    }
}

#[test]
fn completed_browser_playback_closes_browser_and_reports_close_failures() {
    for fail in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let command: serde_json::Value = serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(command["method"], "browser.close");
            let reply = if fail {
                serde_json::json!({"id":command["id"], "type":"error", "message":"fixture close failed"})
            } else { serde_json::json!({"id":command["id"], "type":"success", "result":{}}) };
            socket.send(Message::Text(reply.to_string())).unwrap();
        });
        let (socket, _) = connect(format!("ws://{address}")).unwrap();
        let mut state = State::default();
        state.librewolf_bidi = Some(LibreWolfBidi {socket, next_id:0, context_id:Some("video-tab".into()),
            cached_at:None, cached_video:None, autoplay_attempted:false, fullscreen_attempted_at:None,
            unmatched_url_reported:None});
        assert_eq!(close_finished_librewolf(&mut state), !fail);
        assert_eq!(state.librewolf_bidi.is_none(), !fail);
        assert_eq!(state.playback_error.is_some(), fail);
        peer.join().unwrap();
    }
}

#[test]
fn browser_playback_always_uses_bundled_librewolf() {
    for browser in [serde_json::Value::Null, serde_json::json!(""), serde_json::json!("firefox"),
                    serde_json::json!("/usr/bin/firefox"), serde_json::json!("librewolf")] {
        let mut item: QueueItem = serde_json::from_value(serde_json::json!({
            "url":"https://example.com/video", "browser":browser
        })).unwrap();
        apply_browser_default(&mut item);
        assert_eq!(item.browser.as_deref(), Some("librewolf"));
    }
    let mut local: QueueItem = serde_json::from_value(serde_json::json!({
        "url":"https://example.com/video", "local":true
    })).unwrap();
    apply_browser_default(&mut local);
    assert!(local.browser.is_none());
    assert_eq!(browser_candidates("firefox"), browser_candidates("librewolf"));
}
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
    post(&mut state,"/repeat",r#"{"enabled":true}"#);
    assert!(state.repeat_enabled);
    assert_eq!(mpv_property(&socket,"loop-file"),Some(serde_json::json!("inf")));
    post(&mut state,"/repeat",r#"{"enabled":false}"#);
    assert!(!state.repeat_enabled);
    let buffering = mpv_osd_options().into_iter().find(|option|option.starts_with("--osd-msg1=")).unwrap();
    let buffering = buffering.strip_prefix("--osd-msg1=").unwrap();
    mpv_ipc(&socket,serde_json::json!(["set_property","osd-msg1",buffering])).unwrap();
    let expanded=mpv_ipc(&socket,serde_json::json!(["expand-text",buffering])).unwrap();
    assert_eq!(expanded["data"], "", "buffering OSD must be absent during normal playback");
    mpv_ipc(&socket,serde_json::json!(["set_property","pause",true])).unwrap();
    let visible=mpv_ipc(&socket,serde_json::json!(["expand-text",buffering.replace("paused-for-cache","pause")])).unwrap();
    assert!(visible["data"].as_str().unwrap().starts_with("Buffering... "));
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
    apply_receiver_command(&mut state,&test_config(),"play",&serde_json::json!({})).unwrap();
    assert_eq!(mpv_property(&socket,"pause"),Some(serde_json::json!(false)));
    apply_receiver_command(&mut state,&test_config(),"pause",&serde_json::json!({})).unwrap();
    assert_eq!(mpv_property(&socket,"pause"),Some(serde_json::json!(true)));
    apply_receiver_command(&mut state,&test_config(),"seekTo",&serde_json::json!({"newTime":"2"})).unwrap();
    apply_receiver_command(&mut state,&test_config(),"setVolume",&serde_json::json!({"volume":"30"})).unwrap();
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
    // Before a file is ready, the same persistent OSD shows loading, then
    // disappears once time-pos becomes available (checked above).
    mpv_ipc(&socket,serde_json::json!(["set_property","force-window","immediate"])).unwrap();
    mpv_ipc(&socket,serde_json::json!(["stop"])).unwrap();
    for _ in 0..20 {
        if mpv_property(&socket,"time-pos").and_then(|v|v.as_f64()).is_none(){break;}
        thread::sleep(Duration::from_millis(25));
    }
    let expanded=mpv_ipc(&socket,serde_json::json!(["expand-text",buffering])).unwrap();
    assert_eq!(expanded["data"],"Loading video...");
    assert!(mpv_startup_in_progress(&state));
    assert_eq!(media_status(&mut state).playback_state,"loading");
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

fn test_config() -> Config {
    Config {
        bind: "127.0.0.1:8765".into(), moonlight: "moonlight".into(), moonlight_args: vec![],
        ytdlp_cookies_from_browser: None, cookies_browser: "off".into(), cookies_profile: None,
        remote_origin_servers: vec![], default_screen: None, default_quality: Some("720".into()),
        default_video_codec: "h264".into(), subtitles: false, preferred_language: None, track_preferences: Default::default(),
    }
}

fn post(state: &mut State, path: &str, body: &'static str) {
    let cfg=test_config();
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

