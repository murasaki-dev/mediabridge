//! DIAL discovery and browser/media launch adapters. This is not Google Cast.
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::{BTreeMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SERVICE: &str = "urn:dial-multiscreen-org:service:dial:1";
pub const APPS: &[&str] = &["YouTube", "Twitch", "Netflix"];
const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub friendly_name: String,
    pub uuid: String,
}
impl Default for Settings {
    fn default() -> Self {
        let digest = format!(
            "{:x}",
            Sha1::digest(format!("{:?}-{}", SystemTime::now(), std::process::id()).as_bytes())
        );
        Self {
            enabled: true,
            friendly_name: "MediaBridge".into(),
            uuid: format!(
                "{}-{}-{}-{}-{}",
                &digest[..8],
                &digest[8..12],
                &digest[12..16],
                &digest[16..20],
                &digest[20..32]
            ),
        }
    }
}

#[derive(Serialize)]
pub struct Event {
    received_at_epoch_ms: u64,
    app: String,
    bytes: usize,
    fields: Vec<String>,
    outcome: String,
}

pub struct Discovery {
    path: PathBuf,
    settings: Arc<RwLock<Settings>>,
    error: Arc<Mutex<Option<String>>>,
    listening: Arc<AtomicBool>,
    interfaces: Arc<Mutex<Vec<Ipv4Addr>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    events: VecDeque<Event>,
}

impl Discovery {
    pub fn start(path: PathBuf, bind: SocketAddr) -> Result<Self, String> {
        let settings: Settings = if path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid DIAL settings: {e}"))?
        } else {
            Settings::default()
        };
        validate_name(&settings.friendly_name)?;
        if settings.uuid.len() != 36
            || !settings
                .uuid
                .bytes()
                .all(|c| c.is_ascii_hexdigit() || c == b'-')
        {
            return Err("invalid DIAL device UUID".into());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&settings).unwrap())
            .map_err(|e| e.to_string())?;
        let mut discovery = Self {
            path,
            settings: Arc::new(RwLock::new(settings)),
            error: Arc::new(Mutex::new(None)),
            listening: Arc::new(AtomicBool::new(false)),
            interfaces: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            events: VecDeque::new(),
        };
        let (settings, error, listening, stop, interfaces) = (
            discovery.settings.clone(),
            discovery.error.clone(),
            discovery.listening.clone(),
            discovery.stop.clone(),
            discovery.interfaces.clone(),
        );
        discovery.worker = Some(thread::spawn(move || {
            if let Err(message) = serve_discovery(bind, &settings, &listening, &stop, &interfaces) {
                *error.lock().unwrap() = Some(message);
            }
            listening.store(false, Ordering::Release);
        }));
        Ok(discovery)
    }
    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }
    pub fn update(&mut self, enabled: bool, name: String) -> Result<(), String> {
        validate_name(&name)?;
        let mut next = self.settings();
        next.enabled = enabled;
        next.friendly_name = name;
        std::fs::write(&self.path, serde_json::to_vec_pretty(&next).unwrap())
            .map_err(|e| e.to_string())?;
        *self.settings.write().unwrap() = next;
        Ok(())
    }
    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({"enabled":self.settings().enabled,"friendly_name":self.settings().friendly_name,
            "uuid":self.settings().uuid,"listening":self.listening.load(Ordering::Acquire),
            "error":*self.error.lock().unwrap(),"apps":APPS,"recent_requests":self.events,
            "interfaces":*self.interfaces.lock().unwrap(),
            "google_cast":false,"pairing":"YouTube Lounge receiver; other apps use browser flow"})
    }
    pub fn record(&mut self, app: &str, payload: &str, outcome: &str) {
        let fields = fields(payload)
            .map(|fields| {
                fields
                    .keys()
                    .take(16)
                    .map(|key| {
                        key.chars()
                            .filter(|c| c.is_ascii_alphanumeric() || "_-".contains(*c))
                            .take(64)
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.events.push_front(Event {
            received_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            app: app.into(),
            bytes: payload.len(),
            fields,
            outcome: outcome.into(),
        });
        self.events.truncate(20);
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        Err("DIAL name must be 1–80 bytes without control characters".into())
    } else {
        Ok(())
    }
}
pub fn base_url(bind: SocketAddr, peer: SocketAddr) -> Result<String, String> {
    let ip = if bind.ip().is_unspecified() {
        let socket = UdpSocket::bind(if peer.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .map_err(|e| e.to_string())?;
        socket.connect(peer).map_err(|e| e.to_string())?;
        socket.local_addr().map_err(|e| e.to_string())?.ip()
    } else {
        bind.ip()
    };
    Ok(format!("http://{}", SocketAddr::new(ip, bind.port())))
}
pub fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn device_xml(settings: &Settings) -> String {
    format!(
        "<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><specVersion><major>1</major><minor>0</minor></specVersion><device><deviceType>urn:schemas-upnp-org:device:tvdevice:1</deviceType><friendlyName>{}</friendlyName><manufacturer>MediaBridge</manufacturer><modelName>MediaBridge DIAL</modelName><UDN>uuid:{}</UDN></device></root>",
        xml(&settings.friendly_name),
        xml(&settings.uuid)
    )
}
pub fn app_xml(app: &str, running: bool) -> String {
    format!(
        "<?xml version=\"1.0\"?><service xmlns=\"urn:dial-multiscreen-org:schemas:dial\" dialVer=\"1.7\"><name>{}</name><options allowStop=\"true\"/><state>{}</state>{}</service>",
        xml(app),
        if running { "running" } else { "stopped" },
        if running {
            "<link rel=\"run\" href=\"run\"/>"
        } else {
            ""
        }
    )
}
fn search_delay(packet: &str) -> Option<Duration> {
    let mut lines = packet.lines();
    if !lines
        .next()?
        .trim()
        .eq_ignore_ascii_case("M-SEARCH * HTTP/1.1")
    {
        return None;
    }
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().into()))
        .collect();
    if headers.get("man")?.trim_matches('"') != "ssdp:discover" {
        return None;
    }
    let st = headers.get("st")?;
    if st != SERVICE && st != "ssdp:all" {
        return None;
    }
    let mx = headers.get("mx")?.parse::<u64>().ok()?.clamp(1, 5);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .subsec_nanos() as u64;
    Some(Duration::from_millis(nanos % (mx * 1000)))
}
pub fn search_response(settings: &Settings, base: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=90\r\nEXT:\r\nLOCATION: {base}/dial/device.xml\r\nSERVER: MediaBridge/1.0 UPnP/1.1 DIAL/1.7\r\nST: {SERVICE}\r\nUSN: uuid:{}::{SERVICE}\r\n\r\n",
        settings.uuid
    )
}
fn usable_discovery_ip(ip:Ipv4Addr)->bool {
    !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_multicast() && ip!=Ipv4Addr::BROADCAST
}
fn discovery_interfaces(bind:SocketAddr)->Result<Vec<Ipv4Addr>,String> {
    let IpAddr::V4(ip)=bind.ip() else{return Err("DIAL needs IPv4".into());};
    if !ip.is_unspecified(){return Ok(vec![ip]);}
    let mut addresses=if_addrs::get_if_addrs().map_err(|e|format!("could not enumerate DIAL interfaces: {e}"))?
        .into_iter().filter(|interface|interface.is_oper_up()).filter_map(|interface|match interface.ip(){
            IpAddr::V4(ip) if usable_discovery_ip(ip)=>Some(ip),_=>None,
        }).collect::<Vec<_>>();
    addresses.sort_unstable();addresses.dedup();
    if addresses.is_empty(){return Err("no active IPv4 interface is available for DIAL discovery".into());}
    Ok(addresses)
}
fn notify(socket: &UdpSocket, settings: &Settings, bind: SocketAddr, alive: bool, interfaces:&[Ipv4Addr]) {
    for interface in interfaces {
        if let Err(error)=socket2::SockRef::from(socket).set_multicast_if_v4(interface){
            eprintln!("DIAL: could not select multicast interface {interface}: {error}");continue;
        }
        let base=format!("http://{interface}:{}",bind.port());
        let message = format!(
            "NOTIFY * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nCACHE-CONTROL: max-age=90\r\nLOCATION: {base}/dial/device.xml\r\nNT: {SERVICE}\r\nNTS: ssdp:{}\r\nUSN: uuid:{}::{SERVICE}\r\nSERVER: MediaBridge/1.0 UPnP/1.1 DIAL/1.7\r\n\r\n",
            if alive { "alive" } else { "byebye" },
            settings.uuid
        );
        if let Err(error)=socket.send_to(message.as_bytes(), (GROUP, 1900)){
            eprintln!("DIAL: could not advertise on {interface}: {error}");
        }
    }
}
fn serve_discovery(
    bind: SocketAddr,
    settings: &RwLock<Settings>,
    listening: &AtomicBool,
    stop: &AtomicBool,
    advertised_interfaces: &Mutex<Vec<Ipv4Addr>>,
) -> Result<(), String> {
    if !bind.is_ipv4() || bind.ip().is_loopback() {
        return Err("DIAL discovery needs an IPv4 LAN bind address".into());
    }
    let socket =
        Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| e.to_string())?;
    socket.set_reuse_address(true).map_err(|e| e.to_string())?;
    socket
        .bind(&SocketAddr::from(([0, 0, 0, 0], 1900)).into())
        .map_err(|e| format!("cannot listen on UDP 1900: {e}"))?;
    let socket: UdpSocket = socket.into();
    let mut interfaces=Vec::new();
    for interface in discovery_interfaces(bind)? {
        match socket.join_multicast_v4(&GROUP,&interface){
            Ok(())=>interfaces.push(interface),
            Err(error)=>eprintln!("DIAL: could not join multicast on {interface}: {error}"),
        }
    }
    if interfaces.is_empty(){return Err("cannot join DIAL multicast on any active IPv4 interface".into());}
    *advertised_interfaces.lock().unwrap()=interfaces.clone();
    socket.set_multicast_ttl_v4(2).map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 2048];
    let mut pending = VecDeque::new();
    let mut last = None::<Settings>;
    let mut announced = Instant::now() - Duration::from_secs(31);
    while !stop.load(Ordering::Acquire) {
        let current = settings.read().unwrap().clone();
        if let Some(previous) = last.as_ref().filter(|old| {
            old.enabled && (!current.enabled || old.friendly_name != current.friendly_name)
        }) {
            notify(&socket, previous, bind, false, &interfaces);
            announced = Instant::now() - Duration::from_secs(31);
        }
        listening.store(current.enabled, Ordering::Release);
        if current.enabled
            && (last.as_ref().is_none_or(|old| !old.enabled)
                || announced.elapsed() >= Duration::from_secs(30))
        {
            notify(&socket, &current, bind, true, &interfaces);
            announced = Instant::now();
        }
        last = Some(current.clone());
        match socket.recv_from(&mut buffer) {
            Ok((count, peer)) if current.enabled && peer.port() != 0 && pending.len() < 32 => {
                if let Ok(packet) = std::str::from_utf8(&buffer[..count]) {
                    if let Some(delay) = search_delay(packet) {
                        pending.push_back((Instant::now() + delay, peer));
                    }
                }
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(format!("DIAL discovery socket failed: {error}")),
        }
        let now = Instant::now();
        pending.retain(|(due, peer)| {
            if *due > now {
                return true;
            }
            if current.enabled {
                if let Ok(base) = base_url(bind, *peer) {
                    let _ = socket.send_to(search_response(&current, &base).as_bytes(), peer);
                }
            }
            false
        });
    }
    if let Some(previous) = last.filter(|old| old.enabled) {
        notify(&socket, &previous, bind, false, &interfaces);
    }
    Ok(())
}

fn decode(text: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let input = text.as_bytes();
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'%' => {
                if index + 2 >= input.len() {
                    return Err("invalid percent encoding".into());
                }
                let hex = std::str::from_utf8(&input[index + 1..index + 3])
                    .map_err(|_| "invalid percent encoding")?;
                bytes.push(u8::from_str_radix(hex, 16).map_err(|_| "invalid percent encoding")?);
                index += 3;
            }
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    let value = String::from_utf8(bytes).map_err(|_| "launch data must be UTF-8")?;
    if value.chars().any(char::is_control) {
        return Err("launch data contains control characters".into());
    }
    Ok(value)
}
fn fields(payload: &str) -> Result<BTreeMap<String, String>, String> {
    if payload.len() > 8192 {
        return Err("DIAL payload exceeds 8192 bytes".into());
    }
    let payload = payload.trim();
    let mut output = BTreeMap::new();
    if payload.is_empty() {
        return Ok(output);
    }
    if payload.starts_with('{') {
        let value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| "invalid launch JSON")?;
        for (key, value) in value.as_object().ok_or("launch JSON must be an object")? {
            let value = if let Some(value) = value.as_str() {
                value.to_string()
            } else if value.is_number() {
                value.to_string()
            } else {
                return Err("launch fields must be strings or numbers".into());
            };
            output.insert(key.clone(), value);
        }
    } else if payload.starts_with("http://") || payload.starts_with("https://") {
        output.insert("url".into(), payload.into());
    } else {
        for part in payload.split('&') {
            let (key, value) = part
                .split_once('=')
                .ok_or("unsupported opaque DIAL launch data")?;
            let key = decode(key)?;
            if output.insert(key, decode(value)?).is_some() {
                return Err("duplicate launch field".into());
            }
        }
    }
    if output.len() > 32 {
        return Err("too many launch fields".into());
    }
    Ok(output)
}
#[derive(Debug, PartialEq)]
pub struct Launch {
    pub url: String,
    pub local: bool,
}
pub fn pairing_code(payload: &str) -> Result<Option<String>, String> {
    let fields = fields(payload)?;
    let Some(code) = fields.get("pairingCode") else { return Ok(None); };
    if code.is_empty() || code.len() > 512 || code.chars().any(char::is_control) {
        return Err("invalid YouTube pairing code".into());
    }
    Ok(Some(code.clone()))
}
pub fn launch(app: &str, payload: &str) -> Result<Launch, String> {
    if !APPS.contains(&app) {
        return Err("unknown DIAL application".into());
    }
    let fields = fields(payload)?;
    if let Some(url) = fields
        .get("url")
        .or_else(|| fields.get("contentUrl"))
        .or_else(|| fields.get("content_url"))
    {
        let uri = url
            .parse::<tungstenite::http::Uri>()
            .map_err(|_| "invalid media URL")?;
        let host = uri.host().unwrap_or("").to_ascii_lowercase();
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_some_and(|a| a.as_str().contains('@'))
        {
            return Err("media URL must be public HTTP(S) without credentials".into());
        }
        let allowed = match app {
            "YouTube" => {
                host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com")
            }
            "Twitch" => host == "twitch.tv" || host.ends_with(".twitch.tv"),
            "Netflix" => host == "netflix.com" || host.ends_with(".netflix.com"),
            _ => false,
        };
        if !allowed {
            return Err("URL does not belong to the requested app".into());
        }
        return Ok(Launch {
            url: url.clone(),
            local: match app {
                "YouTube" => crate::youtube_video_id(url).is_some(),
                "Twitch" => {
                    let path = uri.path().trim_matches('/');
                    !path.is_empty()
                        && ![
                            "login",
                            "signup",
                            "settings",
                            "activate",
                            "directory",
                            "search",
                            "downloads",
                            "subscriptions",
                            "wallet",
                            "inventory",
                            "drops",
                            "turbo",
                        ]
                        .iter()
                        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
                }
                _ => false,
            },
        });
    }
    let id = fields
        .get("v")
        .or_else(|| fields.get("videoId"))
        .or_else(|| fields.get("video_id"))
        .or_else(|| fields.get("contentId"));
    if let Some(id) = id {
        let url = match app {
            "YouTube"
                if id.len() == 11
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)) =>
            {
                format!("https://www.youtube.com/watch?v={id}")
            }
            "Twitch" if !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()) => {
                format!("https://www.twitch.tv/videos/{id}")
            }
            "Netflix" if !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()) => {
                format!("https://www.netflix.com/watch/{id}")
            }
            _ => return Err("invalid video/content ID".into()),
        };
        return Ok(Launch {
            url,
            local: app != "Netflix",
        });
    }
    if app == "Twitch" {
        if let Some(channel) = fields.get("channel") {
            if channel.is_empty()
                || channel.len() > 64
                || !channel
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
                return Err("invalid Twitch channel".into());
            }
            return Ok(Launch {
                url: format!("https://www.twitch.tv/{channel}"),
                local: true,
            });
        }
    }
    if app == "YouTube" {
        if let Some(code) = fields.get("pairingCode") {
            if code.is_empty() || code.len() > 512 || code.chars().any(char::is_control) {
                return Err("invalid YouTube pairing code".into());
            }
            return Err("YouTube pairing requires the Lounge receiver adapter".into());
        }
    }
    if !fields.is_empty() {
        return Err("unsupported app-specific launch fields; see DIAL request diagnostics".into());
    }
    Ok(Launch {
        url: match app {
            "YouTube" => "https://www.youtube.com/tv",
            "Twitch" => "https://www.twitch.tv/",
            _ => "https://www.netflix.com/login",
        }
        .into(),
        local: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a live multicast interface specified by MEDIABRIDGE_TEST_DIAL_INTERFACE"]
    fn live_multicast_advertises_on_requested_interface() {
        let ip:Ipv4Addr=std::env::var("MEDIABRIDGE_TEST_DIAL_INTERFACE").expect("test interface IP").parse().unwrap();
        let monitor=Socket::new(Domain::IPV4,Type::DGRAM,Some(Protocol::UDP)).unwrap();
        monitor.set_reuse_address(true).unwrap();
        monitor.bind(&SocketAddr::from(([0,0,0,0],1900)).into()).unwrap();
        let monitor:UdpSocket=monitor.into();
        monitor.join_multicast_v4(&GROUP,&ip).unwrap();
        monitor.set_read_timeout(Some(Duration::from_millis(250))).unwrap();
        let reserve=std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let bind=reserve.local_addr().unwrap();
        let directory=std::env::temp_dir().join(format!("mediabridge-dial-multicast-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let path=directory.join("dial.json");
        let discovery=Discovery::start(path.clone(),bind).unwrap();
        let location=format!("LOCATION: http://{ip}:{}/dial/device.xml",bind.port());
        let deadline=Instant::now()+Duration::from_secs(5);
        let mut observed=false;
        let mut bytes=[0u8;4096];
        while Instant::now()<deadline {
            if let Ok((count,source))=monitor.recv_from(&mut bytes){
                let packet=String::from_utf8_lossy(&bytes[..count]);
                if source.ip()==IpAddr::V4(ip) && packet.contains(&location) && packet.contains("NTS: ssdp:alive") {
                    observed=true;break;
                }
            }
        }
        let joined=discovery.status()["interfaces"].clone();
        drop(discovery);
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(directory).unwrap();
        assert!(observed,"did not receive an announcement on {ip}; joined interfaces: {joined}");
    }
    #[test]
    fn payloads_route_media_to_mpv_and_pairing_to_browser() {
        assert_eq!(
            launch("YouTube", "v=abcdefghijk").unwrap(),
            Launch {
                url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
                local: true
            }
        );
        assert!(
            launch("Twitch", r#"{"channel":"example_channel"}"#)
                .unwrap()
                .local
        );
        assert!(
            launch("YouTube", "https://www.youtube.com/watch?v=abcdefghijk")
                .unwrap()
                .local
        );
        assert!(!launch("YouTube", "https://www.youtube.com/").unwrap().local);
        assert!(
            !launch("Twitch", "https://www.twitch.tv/login")
                .unwrap()
                .local
        );
        assert_eq!(pairing_code("pairingCode=abc%26def&dialLaunch=1&theme=cl").unwrap().as_deref(),Some("abc&def"));
        assert!(launch("YouTube", "pairingCode=abc%26def").is_err());
        assert!(!launch("Netflix", "contentId=123456").unwrap().local);
        for (app, body) in [
            ("Unknown", ""),
            ("YouTube", "v=abc&v=def"),
            ("YouTube", "url=http%3A%2F%2Fyoutube.com.attacker.test%2Fx"),
            ("YouTube", "v=%0Asecret"),
            ("Netflix", "opaque-token"),
            ("Twitch", "channel=%FF"),
        ] {
            assert!(launch(app, body).is_err(), "{app} {body}");
        }
    }
    #[test]
    fn discovery_and_xml_are_protocol_specific() {
        let settings = Settings {
            enabled: true,
            friendly_name: "A & B <TV>".into(),
            uuid: "device-id".into(),
        };
        assert!(device_xml(&settings).contains("A &amp; B &lt;TV&gt;"));
        assert!(
            search_response(&settings, "http://192.168.1.2:8765")
                .contains("LOCATION: http://192.168.1.2:8765/dial/device.xml\r\n")
        );
        assert!(app_xml("YouTube", true).contains("href=\"run\""));
        assert!(
            search_delay(&format!(
                "M-SEARCH * HTTP/1.1\r\nMAN: \"ssdp:discover\"\r\nST: {SERVICE}\r\nMX: 1\r\n\r\n"
            ))
            .is_some()
        );
        assert!(
            search_delay(
                "M-SEARCH * HTTP/1.1\r\nMAN: \"ssdp:discover\"\r\nST: _googlecast._tcp\r\nMX: 1\r\n"
            )
            .is_none()
        );
    }
}
