use super::{Probe, Process};
use std::{
    mem::size_of,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::ToolHelp::*, Threading::*},
    UI::WindowsAndMessaging::*,
};

fn process(pid: u32) -> Option<Process> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut path = [0u16; 32768];
        let mut len = path.len() as u32;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut len) != 0
            && GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        windows_sys::Win32::System::Time::FileTimeToSystemTime(&created, &mut utc);
        windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime(
            std::ptr::null(),
            &utc,
            &mut local,
        );
        ok.then(|| Process {
            pid,
            started: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
            started_local: format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute, local.wSecond
            ),
            exe: String::from_utf16_lossy(&path[..len as usize]),
            app_id: None,
        })
    }
}

struct Windows {
    pids: Vec<u32>,
    picture: bool,
    handles: Vec<HWND>,
}
unsafe extern "system" fn window(hwnd: HWND, arg: LPARAM) -> i32 {
    unsafe {
        let state = &mut *(arg as *mut Windows);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if !state.pids.contains(&pid) {
            return 1;
        }
        state.handles.push(hwnd);
        let mut title = [0u16; 1024];
        let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        let title = String::from_utf16_lossy(&title[..len.max(0) as usize]).to_lowercase();
        // Exists, even if minimized or covered by the game. Never match fullscreen alone.
        if title.contains("big picture") || title == "steam gamepad ui" {
            state.picture = true;
        }
        1
    }
}
fn windows(pids: Vec<u32>) -> Result<Windows, String> {
    let mut state = Windows {
        pids,
        picture: false,
        handles: vec![],
    };
    if unsafe { EnumWindows(Some(window), &mut state as *mut _ as LPARAM) } == 0 {
        return Err("could not enumerate desktop windows".into());
    }
    Ok(state)
}
pub(super) fn probe() -> Result<Probe, String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err("could not enumerate host processes".into());
    }
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut processes = vec![];
    unsafe {
        let mut desktop_session = 0;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut desktop_session) == 0 {
            CloseHandle(snapshot);
            return Err("could not determine the desktop session".into());
        }
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let mut session = u32::MAX;
            if ProcessIdToSessionId(entry.th32ProcessID, &mut session) != 0
                && session == desktop_session
            {
                if let Some(p) = process(entry.th32ProcessID) {
                    processes.push(p);
                }
            }
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    let steam = processes.iter().find(|p| {
        PathBuf::from(&p.exe)
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("steam.exe"))
    });
    let root = steam.and_then(|p| PathBuf::from(&p.exe).parent().map(|p| p.to_path_buf()));
    let pids = processes
        .iter()
        .filter(|p| {
            root.as_ref()
                .is_some_and(|root| PathBuf::from(&p.exe).starts_with(root))
                && (p.exe.to_lowercase().ends_with("\\steam.exe")
                    || p.exe.to_lowercase().ends_with("\\steamwebhelper.exe"))
        })
        .map(|p| p.pid)
        .collect();
    let picture = windows(pids)?.picture;
    Ok(Probe {
        running: steam.is_some(),
        root,
        big_picture_open: Some(picture),
        processes,
    })
}
fn uri(value: &str) -> Result<(), String> {
    let mut child = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", value])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err("Windows could not dispatch the Steam command".into())
            };
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Steam command dispatch timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
pub(super) fn open() -> Result<(), String> {
    uri("steam://open/bigpicture")
}
pub(super) fn close_picture() -> Result<(), String> {
    uri("steam://close/bigpicture")
}
pub(super) fn stop(p: &Process, force: bool) -> Result<(), String> {
    if !process(p.pid).is_some_and(|live| live.started == p.started && live.exe == p.exe) {
        return Ok(());
    }
    if !force {
        for hwnd in windows(vec![p.pid])?.handles {
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
        return Ok(());
    }
    unsafe {
        let h = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            p.pid,
        );
        if h.is_null() {
            return Err(format!("could not end game process {}", p.pid));
        }
        // Revalidate identity on the same handle used for termination.
        let mut c = FILETIME::default();
        let mut e = FILETIME::default();
        let mut k = FILETIME::default();
        let mut u = FILETIME::default();
        let valid = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0
            && (((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64) == p.started;
        let ok = !valid || TerminateProcess(h, 1) != 0;
        CloseHandle(h);
        if ok {
            Ok(())
        } else {
            Err(format!("could not end game process {}", p.pid))
        }
    }
}
pub(super) fn hide() -> Result<(), String> {
    let p = probe()?;
    let pids = p
        .processes
        .iter()
        .filter(|process| {
            p.root
                .as_ref()
                .is_some_and(|r| PathBuf::from(&process.exe).starts_with(r))
                && (process.exe.to_lowercase().ends_with("\\steam.exe")
                    || process.exe.to_lowercase().ends_with("\\steamwebhelper.exe"))
        })
        .map(|p| p.pid)
        .collect();
    for hwnd in windows(pids)?.handles {
        unsafe {
            ShowWindowAsync(hwnd, SW_HIDE);
        }
    }
    Ok(())
}
