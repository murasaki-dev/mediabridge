//! Independent implementation of the YouTube Lounge receiver wire protocol.
//! Tokens stay in memory; network errors deliberately omit request URLs/bodies.
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, SyncSender},
};
use std::thread;
use std::time::Duration;

const BASE: &str = "https://www.youtube.com/api/lounge";
const LIMIT: usize = 262144;
pub type Message = (String, Value);

#[derive(Default)]
struct SenderConnections {
    active: BTreeSet<String>,
    known: BTreeSet<String>,
}
fn payload_value(value: &Value) -> Option<Value> {
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).ok()
    } else {
        Some(value.clone())
    }
}
impl SenderConnections {
    fn update(&mut self, command: &str, args: &Value) -> (bool, Option<&'static str>) {
        if command == "remoteConnected" {
            if let Some(id) = args["id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 512)
            {
                self.active.insert(id.to_string());
                self.known.insert(id.to_string());
            }
        } else if command == "remoteDisconnected" {
            // This notification alone cannot distinguish UI action from loss.
            if let Some(id) = args["id"].as_str() {
                self.active.remove(id);
            }
            return (false, Some("unknown"));
        } else if command == "loungeStatus" {
            let Some(devices) = payload_value(&args["devices"]) else {
                return (false, None);
            };
            let Some(devices) = devices.as_array() else {
                return (false, None);
            };
            if devices.len() > 256 {
                return (false, None);
            }
            let mut active = BTreeSet::new();
            for device in devices
                .iter()
                .filter(|device| device["type"] == "REMOTE_CONTROL")
            {
                let Some(id) = device["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 512)
                else {
                    return (false, None);
                };
                active.insert(id.to_string());
            }
            let event = payload_value(&args["connectionEventDetails"]);
            let leaving = event
                .as_ref()
                .and_then(|event| event["deviceId"].as_str())
                .is_some_and(|id| self.known.contains(id) && !active.contains(id));
            let explicit = event
                .as_ref()
                .is_some_and(|event| event["ui"] == "true" || event["ui"] == true);
            let reason = if leaving {
                Some(if explicit {
                    "manual"
                } else {
                    "implicit_or_unknown"
                })
            } else {
                None
            };
            self.active = active;
            self.known.extend(self.active.iter().cloned());
            // Keep identity tracking bounded without publishing identifiers.
            while self.known.len() > 256 {
                self.known.pop_first();
            }
            return (leaving && explicit && self.active.is_empty(), reason);
        }
        (false, None)
    }
}

pub struct Lounge {
    pub incoming: Receiver<Message>,
    outgoing: SyncSender<Message>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Value>>,
    senders: Mutex<SenderConnections>,
}
impl Lounge {
    pub fn start(code: String, name: String, device: String) -> Self {
        Self::start_at(code, name, device, BASE.to_string())
    }
    fn start_at(code: String, name: String, device: String, base: String) -> Self {
        let (tx, incoming) = mpsc::sync_channel(64);
        let (outgoing, rx) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(json!({"phase":"connecting","error":null})));
        let worker_stop = stop.clone();
        let worker_status = status.clone();
        thread::spawn(move || {
            if let Err(error) = run(
                &base,
                code,
                name,
                device,
                tx,
                rx,
                &worker_stop,
                &worker_status,
            ) {
                if !worker_stop.load(Ordering::Acquire) {
                    *worker_status.lock().unwrap() = json!({"phase":"error","error":error});
                    eprintln!("YouTube receiver: {error}");
                    worker_stop.store(true, Ordering::Release);
                }
            }
        });
        Self {
            incoming,
            outgoing,
            stop,
            status,
            senders: Mutex::new(SenderConnections::default()),
        }
    }
    pub fn status(&self) -> Value {
        self.status.lock().unwrap().clone()
    }
    pub fn running(&self) -> bool {
        self.status()["phase"] != "error"
    }
    pub fn send(&self, command: &str, args: Value) {
        let _ = self.outgoing.try_send((command.to_string(), args));
    }
    pub fn received(&self, command: &str, args: &Value) -> bool {
        let mut senders = self.senders.lock().unwrap();
        let (manual_stop, disconnect) = senders.update(command, args);
        let mut status = self.status.lock().unwrap();
        let count = status["commands_received"].as_u64().unwrap_or(0) + 1;
        status["commands_received"] = count.into();
        status["last_command"] = command
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(64)
            .collect::<String>()
            .into();
        if matches!(
            command,
            "remoteConnected" | "setPlaylist" | "setVideo" | "getNowPlaying" | "getVolume"
        ) {
            status["phone_connected"] = true.into();
            if status["phase"] != "error" {
                status["phase"] = "connected".into();
            }
        } else if command == "remoteDisconnected" {
            status["phone_connected"] = false.into();
            if status["phase"] != "error" {
                status["phase"] = "waiting_for_phone".into();
            }
        }
        if command == "loungeStatus" {
            status["connected_senders"] = senders.active.len().into();
            status["phone_connected"] = (!senders.active.is_empty()).into();
            if status["phase"] != "error" {
                status["phase"] = if senders.active.is_empty() {
                    "waiting_for_phone"
                } else {
                    "connected"
                }
                .into();
            }
        }
        if let Some(reason) = disconnect {
            status["last_disconnect"] = reason.into();
        }
        manual_stop
    }
}
impl Drop for Lounge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn checked(response: Result<Response, reqwest::Error>, phase: &str) -> Result<Response, String> {
    let response = response.map_err(|_| format!("{phase}: network request failed"))?;
    if !response.status().is_success() {
        return Err(format!("{phase}: HTTP {}", response.status().as_u16()));
    }
    Ok(response)
}
fn small(response: Response, phase: &str) -> Result<String, String> {
    let mut body = String::new();
    response
        .take((LIMIT + 1) as u64)
        .read_to_string(&mut body)
        .map_err(|_| format!("{phase}: unreadable response"))?;
    if body.len() > LIMIT {
        return Err(format!("{phase}: oversized response"));
    }
    Ok(body)
}

// BrowserChannel frames consist of a decimal length, newline, then a JSON batch.
fn frame(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut length = String::new();
    loop {
        length.clear();
        if (&mut *reader)
            .take(32)
            .read_line(&mut length)
            .map_err(|_| "command channel read failed")?
            == 0
        {
            return Ok(None);
        }
        if length.trim().is_empty() {
            continue;
        }
        break;
    }
    let length: usize = length
        .trim()
        .parse()
        .map_err(|_| "invalid command frame length")?;
    if length > LIMIT {
        return Err("command frame exceeds limit".into());
    }
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "incomplete command frame")?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "invalid command frame JSON".into())
}

fn dispatch(
    batch: Value,
    aid: &AtomicI64,
    tx: &SyncSender<Message>,
    sid: &mut String,
    gs: &mut String,
) -> Result<(), String> {
    for row in batch.as_array().ok_or("invalid command batch")? {
        let index = row[0].as_i64().ok_or("invalid command sequence")?;
        if index <= aid.load(Ordering::Acquire) {
            continue;
        }
        let command = row[1][0].as_str().ok_or("invalid command name")?;
        match command {
            "c" => *sid = row[1][1].as_str().ok_or("missing channel SID")?.to_string(),
            "S" => {
                *gs = row[1][1]
                    .as_str()
                    .ok_or("missing channel session")?
                    .to_string()
            }
            "stop" | "close" | "loungeScreenDisconnected" => {
                return Err("YouTube ended the receiver session; reconnect from the phone".into());
            }
            "noop" => {}
            // Never log remote arguments: these can contain account identifiers.
            _ => tx
                .try_send((command.to_string(), row[1][1].clone()))
                .map_err(|_| "receiver command queue full or closed")?,
        }
        aid.store(index, Ordering::Release);
    }
    Ok(())
}

fn run(
    base: &str,
    code: String,
    name: String,
    device: String,
    incoming: SyncSender<Message>,
    outgoing: Receiver<Message>,
    stop: &Arc<AtomicBool>,
    status: &Arc<Mutex<Value>>,
) -> Result<(), String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("MediaBridge YouTube Receiver/1.2")
        .build()
        .map_err(|_| "could not initialize receiver HTTPS client")?;
    let screen = small(
        checked(
            client
                .get(format!("{base}/pairing/generate_screen_id"))
                .timeout(Duration::from_secs(15))
                .send(),
            "screen registration",
        )?,
        "screen registration",
    )?;
    let screen = screen.trim().trim_matches('"');
    if screen.is_empty() || screen.len() > 512 || screen.chars().any(char::is_control) {
        return Err("YouTube returned an invalid screen identifier".into());
    }
    let token_body = small(
        checked(
            client
                .post(format!("{base}/pairing/get_lounge_token_batch"))
                .form(&[("screen_ids", screen)])
                .timeout(Duration::from_secs(15))
                .send(),
            "receiver token",
        )?,
        "receiver token",
    )?;
    let token_json: Value =
        serde_json::from_str(&token_body).map_err(|_| "invalid receiver token response")?;
    let token = token_json["screens"][0]["loungeToken"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("missing receiver token")?;
    let common = vec![
        ("device", "LOUNGE_SCREEN".to_string()),
        ("id", device.clone()),
        ("name", name.clone()),
        ("app", "youtube-desktop".into()),
        ("theme", "cl".into()),
        ("capabilities", "dsp".into()),
        ("cst", "m".into()),
        ("deviceInfo", json!({"brand":"MediaBridge","model":"MediaBridge","year":0,"os":std::env::consts::OS,"osVersion":"","chipset":"","clientName":"TVHTML5","dialAdditionalDataSupportLevel":"unsupported"}).to_string()),
        ("mdxVersion", "2".into()),
        ("VER", "8".into()),
        ("v", "2".into()),
        ("CVER", "1".into()),
        ("loungeIdToken", token.to_string()),
    ];
    let mut initial = common.clone();
    initial.push(("RID", "1".into()));
    let response = checked(
        client
            .post(format!("{base}/bc/bind"))
            .query(&initial)
            .form(&[("count", "0")])
            .timeout(Duration::from_secs(15))
            .send(),
        "receiver handshake",
    )?;
    let aid = Arc::new(AtomicI64::new(-1));
    let mut sid = String::new();
    let mut gs = String::new();
    let mut reader = BufReader::new(response);
    while sid.is_empty() || gs.is_empty() {
        let batch =
            frame(&mut reader)?.ok_or("receiver handshake ended before session was established")?;
        dispatch(batch, &aid, &incoming, &mut sid, &mut gs)?;
    }
    drop(reader);
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    // Set up the incoming RPC stream before registering the phone's code.
    let mut polling = common.clone();
    polling.extend([
        ("RID", "rpc".into()),
        ("SID", sid.clone()),
        ("gsessionid", gs.clone()),
        ("CI", "0".into()),
        ("TYPE", "xmlhttp".into()),
        ("AID", aid.load(Ordering::Acquire).to_string()),
    ]);
    let first_poll = checked(
        client.get(format!("{base}/bc/bind")).query(&polling).send(),
        "receiver command channel",
    )?;
    let read_stop = stop.clone();
    let read_status = status.clone();
    let read_aid = aid.clone();
    let read_client = client.clone();
    let read_base = base.to_string();
    let read_tx = incoming;
    thread::spawn(move || {
        let mut next = Some(first_poll);
        let mut failures = 0;
        while !read_stop.load(Ordering::Acquire) {
            polling.retain(|(key, _)| *key != "AID");
            polling.push(("AID", read_aid.load(Ordering::Acquire).to_string()));
            let response = next.take().map(Ok).unwrap_or_else(|| {
                checked(
                    read_client
                        .get(format!("{read_base}/bc/bind"))
                        .query(&polling)
                        .send(),
                    "receiver command channel",
                )
            });
            let result = response.and_then(|response| {
                let mut reader = BufReader::new(response);
                while !read_stop.load(Ordering::Acquire) {
                    let Some(batch) = frame(&mut reader)? else {
                        break;
                    };
                    dispatch(
                        batch,
                        &read_aid,
                        &read_tx,
                        &mut String::new(),
                        &mut String::new(),
                    )?;
                    failures = 0;
                }
                Ok(())
            });
            if let Err(error) = result {
                failures += 1;
                if failures >= 3
                    || error.contains("ended the receiver session")
                    || error.contains("HTTP 4")
                {
                    *read_status.lock().unwrap() = json!({"phase":"error","error":error});
                    read_stop.store(true, Ordering::Release);
                    break;
                }
                // Bounded delay, interruptible on stop.
                for _ in 0..10 {
                    if read_stop.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    if !code.is_empty() {
        checked(
            client
                .post(format!("{base}/pairing/register_pairing_code"))
                .form(&[
                    ("access_type", "permanent"),
                    ("app", "youtube-desktop"),
                    ("device_id", &device),
                    ("pairing_code", &code),
                    ("screen_id", screen),
                    ("screen_name", &name),
                ])
                .timeout(Duration::from_secs(15))
                .send(),
            "phone pairing",
        )?;
    }
    if !stop.load(Ordering::Acquire) {
        let mut current = status.lock().unwrap();
        current["phase"] = if current["phone_connected"] == true {
            "connected"
        } else {
            "waiting_for_phone"
        }
        .into();
        current["error"] = Value::Null;
    }
    let mut rid = 2u64;
    let mut offset = 0u64;
    while !stop.load(Ordering::Acquire) {
        let message = match outgoing.recv_timeout(Duration::from_millis(200)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        let mut messages = vec![message];
        messages.extend(outgoing.try_iter().take(15));
        let count = messages.len();
        let mut form = vec![
            ("count".to_string(), count.to_string()),
            ("ofs".into(), offset.to_string()),
        ];
        for (index, (command, args)) in messages.into_iter().enumerate() {
            form.push((format!("req{index}__sc"), command));
            if let Some(args) = args.as_object() {
                for (key, value) in args {
                    form.push((
                        format!("req{index}_{key}"),
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string()),
                    ));
                }
            }
        }
        let mut query = common.clone();
        query.extend([
            ("SID", sid.clone()),
            ("gsessionid", gs.clone()),
            ("RID", rid.to_string()),
            ("AID", aid.load(Ordering::Acquire).to_string()),
        ]);
        checked(
            client
                .post(format!("{base}/bc/bind"))
                .query(&query)
                .form(&form)
                .timeout(Duration::from_secs(15))
                .send(),
            "playback feedback",
        )?;
        rid += 1;
        offset += count as u64;
        let mut current_status = status.lock().unwrap();
        if current_status["phase"] != "error" {
            current_status["messages_sent"] = offset.into();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sender_status(ids: &[&str], event: Value) -> Value {
        let devices = ids
            .iter()
            .map(|id| json!({"id":id,"type":"REMOTE_CONTROL"}))
            .collect::<Vec<_>>();
        json!({"devices":serde_json::to_string(&devices).unwrap(),"connectionEventDetails":event.to_string()})
    }
    #[test]
    fn only_known_last_sender_explicit_ui_disconnect_stops() {
        let mut senders = SenderConnections::default();
        assert!(
            !senders
                .update("loungeStatus", &sender_status(&["a", "b"], Value::Null))
                .0
        );
        assert!(
            !senders
                .update(
                    "loungeStatus",
                    &sender_status(&["b"], json!({"deviceId":"a","ui":"true"}))
                )
                .0
        );
        assert!(
            senders
                .update(
                    "loungeStatus",
                    &sender_status(&[], json!({"deviceId":"b","ui":"true"}))
                )
                .0
        );
        assert!(
            !senders
                .update(
                    "loungeStatus",
                    &sender_status(&[], json!({"deviceId":"unknown","ui":"true"}))
                )
                .0
        );
    }
    #[test]
    fn phone_loss_legacy_disconnect_and_malformed_status_do_not_stop() {
        for event in [
            json!({"deviceId":"a","ui":"false"}),
            json!({"deviceId":"a"}),
            Value::Null,
        ] {
            let mut senders = SenderConnections::default();
            senders.update("remoteConnected", &json!({"id":"a"}));
            assert!(!senders.update("loungeStatus", &sender_status(&[], event)).0);
        }
        let mut senders = SenderConnections::default();
        senders.update("remoteConnected", &json!({"id":"a"}));
        assert!(
            !senders
                .update("remoteDisconnected", &json!({"id":"a","ui":"true"}))
                .0
        );
        for devices in [
            json!("not JSON"),
            json!({}),
            json!([{"type":"REMOTE_CONTROL"}]),
        ] {
            assert!(!senders.update("loungeStatus",&json!({"devices":devices,"connectionEventDetails":{"deviceId":"a","ui":true}})).0);
        }
        assert!(
            !senders
                .update(
                    "loungeStatus",
                    &sender_status(&["a"], json!({"deviceId":"a","ui":"true"}))
                )
                .0
        );
        // A later authoritative UI event can still identify a legacy sender.
        assert!(
            senders
                .update(
                    "loungeStatus",
                    &sender_status(&[], json!({"deviceId":"a","ui":"true"}))
                )
                .0
        );
    }
    #[test]
    #[ignore = "requires live YouTube HTTPS access; uses an isolated synthetic pairing code"]
    fn live_pairing_registration_is_resolvable_by_sender() {
        use sha1::{Digest, Sha1};
        let value = format!(
            "{:x}",
            Sha1::digest(
                format!("{:?}-{}", std::time::SystemTime::now(), std::process::id()).as_bytes()
            )
        );
        let code = format!(
            "{}-{}-{}-{}-{}",
            &value[..8],
            &value[8..12],
            &value[12..16],
            &value[16..20],
            &value[20..32]
        );
        let receiver = Lounge::start(
            code.clone(),
            "MediaBridge pairing test".into(),
            code.clone(),
        );
        for _ in 0..250 {
            let status = receiver.status();
            assert_ne!(status["phase"], "error", "{status}");
            if status["phase"] == "waiting_for_phone" {
                let client = Client::builder()
                    .timeout(Duration::from_secs(15))
                    .build()
                    .unwrap();
                let response = checked(
                    client
                        .post(format!("{BASE}/pairing/get_screen"))
                        .form(&[("pairing_code", &code)])
                        .send(),
                    "sender pairing lookup",
                )
                .unwrap();
                let body = small(response, "sender pairing lookup").unwrap();
                let data: Value =
                    serde_json::from_str(&body).expect("sender lookup must return JSON");
                assert!(
                    data["screen"]["screenId"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()),
                    "sender cannot resolve paired receiver"
                );
                assert!(
                    data["screen"]["loungeToken"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()),
                    "sender did not receive a lounge token"
                );
                assert_eq!(
                    data["screen"]["name"].as_str(),
                    Some("MediaBridge pairing test")
                );
                let token = data["screen"]["loungeToken"].as_str().unwrap();
                let common = vec![
                    ("device", "REMOTE_CONTROL".to_string()),
                    ("id", format!("sender-{code}")),
                    ("name", "MediaBridge sender test".into()),
                    ("app", "youtube-desktop".into()),
                    ("VER", "8".into()),
                    ("CVER", "1".into()),
                    ("loungeIdToken", token.to_string()),
                ];
                let mut initial = common.clone();
                initial.push(("RID", "1".into()));
                let response = checked(
                    client
                        .post(format!("{BASE}/bc/bind"))
                        .query(&initial)
                        .form(&[("count", "0")])
                        .send(),
                    "sender handshake",
                )
                .unwrap();
                let (tx, _rx) = mpsc::sync_channel(64);
                let aid = AtomicI64::new(-1);
                let mut sid = String::new();
                let mut gs = String::new();
                let mut reader = BufReader::new(response);
                while sid.is_empty() || gs.is_empty() {
                    let batch = frame(&mut reader)
                        .unwrap()
                        .expect("sender handshake response");
                    dispatch(batch, &aid, &tx, &mut sid, &mut gs).unwrap();
                }
                let mut query = common;
                query.extend([
                    ("SID", sid),
                    ("gsessionid", gs),
                    ("RID", "2".into()),
                    ("AID", aid.load(Ordering::Acquire).to_string()),
                ]);
                checked(
                    client
                        .post(format!("{BASE}/bc/bind"))
                        .query(&query)
                        .form(&[
                            ("count", "1"),
                            ("ofs", "0"),
                            ("req0__sc", "setPlaylist"),
                            ("req0_videoIds", "abcdefghijk"),
                            ("req0_currentIndex", "0"),
                            ("req0_currentTime", "0"),
                        ])
                        .send(),
                    "sender playlist command",
                )
                .unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while std::time::Instant::now() < deadline {
                    if let Ok((command, args)) =
                        receiver.incoming.recv_timeout(Duration::from_millis(200))
                    {
                        if command == "setPlaylist" {
                            assert_eq!(args["videoIds"], "abcdefghijk");
                            return;
                        }
                    }
                }
                panic!("receiver did not receive the live sender command");
            }
            thread::sleep(Duration::from_millis(200));
        }
        panic!("pairing registration timed out");
    }
    #[test]
    fn mock_receiver_registers_pairing_receives_commands_and_sends_feedback() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let (events_tx, events) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = done.clone();
        let worker = thread::spawn(move || {
            while !worker_done.load(Ordering::Acquire) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(100)).unwrap()
                else {
                    continue;
                };
                let url =
                    reqwest::Url::parse(&format!("http://{address}{}", request.url())).unwrap();
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let fields = reqwest::Url::parse(&format!("http://fixture/?{body}"))
                    .unwrap()
                    .query_pairs()
                    .into_owned()
                    .collect::<std::collections::BTreeMap<_, _>>();
                let response = if url.path().ends_with("generate_screen_id") {
                    "test-screen".to_string()
                } else if url.path().ends_with("get_lounge_token_batch") {
                    assert_eq!(fields["screen_ids"], "test-screen");
                    json!({"screens":[{"loungeToken":"SECRET_TOKEN"}]}).to_string()
                } else if url.path().ends_with("register_pairing_code") {
                    assert_eq!(fields["pairing_code"], "PRIVATE_PAIRING_CODE");
                    assert_eq!(fields["screen_id"], "test-screen");
                    assert_eq!(fields["app"], "youtube-desktop");
                    assert_eq!(fields["device_id"], "test-device");
                    events_tx.send("paired").unwrap();
                    String::new()
                } else if request.method() == &tiny_http::Method::Get {
                    let query = url
                        .query_pairs()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    let message = if query["AID"] == "1" {
                        json!([[2,["setPlaylist",{"videoIds":"abcdefghijk,lmnopqrstuv","currentIndex":"0","currentTime":"12"}]]])
                    } else {
                        json!([[3, ["noop"]]])
                    };
                    thread::sleep(Duration::from_millis(100));
                    let text = message.to_string();
                    format!("{}\n{text}", text.len())
                } else if fields.get("count").is_some_and(|count| count == "0") {
                    let text =
                        json!([[0, ["c", "TEST_SID"]], [1, ["S", "TEST_GSESSION"]]]).to_string();
                    format!("{}\n{text}", text.len())
                } else {
                    assert_eq!(fields["req0__sc"], "onStateChange");
                    assert_eq!(fields["req0_state"], "1");
                    events_tx.send("feedback").unwrap();
                    String::new()
                };
                let _ = request.respond(tiny_http::Response::from_string(response));
            }
        });
        let receiver = Lounge::start_at(
            "PRIVATE_PAIRING_CODE".into(),
            "Test receiver".into(),
            "test-device".into(),
            format!("http://{address}"),
        );
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).unwrap(),
            "paired"
        );
        let (command, args) = receiver
            .incoming
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(command, "setPlaylist");
        receiver.received(&command, &args);
        assert_eq!(receiver.status()["phase"], "connected");
        assert_eq!(receiver.status()["commands_received"], 1);
        assert_eq!(args["currentTime"], "12");
        receiver.send("onStateChange", json!({"state":"1"}));
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).unwrap(),
            "feedback"
        );
        assert!(
            !receiver
                .status()
                .to_string()
                .contains("PRIVATE_PAIRING_CODE")
        );
        assert!(!receiver.status().to_string().contains("SECRET_TOKEN"));
        drop(receiver);
        done.store(true, Ordering::Release);
        worker.join().unwrap();
    }
    #[test]
    #[ignore = "requires live YouTube HTTPS access; does not pair a phone"]
    fn live_receiver_handshake() {
        let receiver = Lounge::start(
            String::new(),
            "MediaBridge protocol test".into(),
            format!("mediabridge-test-{}", std::process::id()),
        );
        for _ in 0..250 {
            let status = receiver.status();
            if status["phase"] == "waiting_for_phone" {
                receiver.send("onStateChange", json!({"state":"-1","currentTime":"0"}));
                for _ in 0..100 {
                    let feedback = receiver.status();
                    assert_ne!(feedback["phase"], "error", "{feedback}");
                    if feedback["messages_sent"].as_u64().unwrap_or(0) > 0 {
                        return;
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                panic!("receiver feedback timed out");
            }
            assert_ne!(status["phase"], "error", "{status}");
            thread::sleep(Duration::from_millis(200));
        }
        panic!("receiver handshake timed out: {}", receiver.status());
    }
    #[test]
    fn frames_are_bounded_and_commands_deduplicated() {
        let data =
            json!([[0,["c","sid"]],[1,["S","gs"]],[2,["setPlaylist",{"videoIds":"abcdefghijk"}]]]);
        let text = data.to_string();
        let mut reader = std::io::Cursor::new(format!("{}\n{text}", text.len()));
        let batch = frame(&mut reader).unwrap().unwrap();
        let (tx, rx) = mpsc::sync_channel(4);
        let aid = AtomicI64::new(-1);
        let mut sid = String::new();
        let mut gs = String::new();
        dispatch(batch.clone(), &aid, &tx, &mut sid, &mut gs).unwrap();
        dispatch(batch, &aid, &tx, &mut sid, &mut gs).unwrap();
        assert_eq!((sid.as_str(), gs.as_str()), ("sid", "gs"));
        assert_eq!(rx.try_iter().count(), 1);
        assert!(frame(&mut std::io::Cursor::new("999999999\n")).is_err());
        assert!(frame(&mut std::io::Cursor::new("2\n[")).is_err());
    }
}
