use super::{Probe, Process};
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn run(action: &str, process: Option<&Process>) -> Result<String, String> {
    // Distrobox's PID namespace is not the desktop host's namespace.
    let boxed = std::env::var_os("CONTAINER_ID").is_some()
        || std::path::Path::new("/run/.containerenv").exists();
    let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
        // Requires an explicit org.freedesktop.Flatpak host-execution permission.
        // The experimental package does not grant that permission by default.
        let mut command = Command::new("flatpak-spawn");
        command.args(["--host", "python3"]);
        command
    } else if boxed {
        let mut command = Command::new("distrobox-host-exec");
        command.arg("python3");
        command
    } else {
        Command::new("python3")
    };
    command.args(["-c", include_str!("steam_linux.py"), action]);
    if let Some(p) = process {
        command
            .arg(p.pid.to_string())
            .arg(p.started.to_string())
            .arg(&p.exe);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|e| format!("Steam host helper needs Python 3 and host access: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Steam helper output unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut data = String::new();
        let _ = stdout.take(8 * 1024 * 1024).read_to_string(&mut data);
        data
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let data = reader.join().unwrap_or_default();
            if status.success() {
                return Ok(data);
            }
            return Err(format!("Steam host helper failed: {}", data.trim()));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Steam host helper timed out".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
pub(super) fn probe() -> Result<Probe, String> {
    serde_json::from_str(&run("probe", None)?).map_err(|e| format!("invalid Steam host probe: {e}"))
}
pub(super) fn open() -> Result<(), String> {
    run("open", None).map(|_| ())
}
pub(super) fn close_picture() -> Result<(), String> {
    run("close", None).map(|_| ())
}
pub(super) fn hide() -> Result<(), String> {
    run("hide", None).map(|_| ())
}
pub(super) fn stop(p: &Process, force: bool) -> Result<(), String> {
    run(if force { "kill" } else { "stop" }, Some(p)).map(|_| ())
}
