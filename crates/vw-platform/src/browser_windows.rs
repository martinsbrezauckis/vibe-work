//! Windows browser observation. Uses process snapshots and top-level windows; called by `browser_process`.
//! Only visible, unowned windows of this session's exact browser executable count as running.
use super::Browser;

fn matches_process(browser: Browser, name: &str, session: u32, own_session: u32) -> bool {
    let executable = if browser == Browser::Edge {
        "msedge.exe"
    } else {
        "chrome.exe"
    };
    session == own_session && name.eq_ignore_ascii_case(executable)
}

fn main_window(pid: u32, visible: bool, owned: bool, candidates: &[u32]) -> bool {
    visible && !owned && candidates.contains(&pid)
}

#[cfg(windows)]
pub(super) fn inspect(browser: Browser) -> super::ProcessState {
    match native::query(browser) {
        Ok(true) => super::ProcessState::Running,
        Ok(false) => super::ProcessState::Stopped,
        Err(_) => super::ProcessState::Unknown,
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod native {
    use super::{Browser, main_window, matches_process};
    use ::windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE, HWND, LPARAM};
    use ::windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use ::windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
    };
    use ::windows::core::{BOOL, HRESULT, Result};

    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            // SAFETY: this is our one owned snapshot handle, closed exactly once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    fn session(pid: u32) -> Result<u32> {
        let mut id = 0;
        // SAFETY: a live writable out pointer; no process handle or borrowed memory is retained.
        unsafe { ProcessIdToSessionId(pid, &raw mut id) }?;
        Ok(id)
    }
    struct Windows {
        candidates: Vec<u32>,
        found: bool,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
        // SAFETY: query passes its live Windows value for this synchronous enumeration only.
        let state = unsafe { &mut *(param.0 as *mut Windows) };
        let mut pid = 0;
        // SAFETY: read-only queries on an enumerated HWND and a live PID out pointer. A closed window is ignored.
        let (visible, owned) = unsafe {
            GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
            (
                IsWindowVisible(hwnd).as_bool(),
                GetWindow(hwnd, GW_OWNER).is_ok(),
            )
        };
        state.found |= main_window(pid, visible, owned, &state.candidates);
        BOOL(1)
    }

    pub(super) fn query(browser: Browser) -> Result<bool> {
        let own_session = session(std::process::id())?;
        // SAFETY: a read-only process snapshot with no module/heap flags or borrowed pointers.
        let snapshot = Snapshot(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?);
        let mut entry = PROCESSENTRY32W {
            dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(u32::MAX),
            ..Default::default()
        };
        let mut candidates = Vec::new();
        // SAFETY: our live snapshot and correctly sized writable entry; nothing escapes the call.
        let mut next = unsafe { Process32FirstW(snapshot.0, &raw mut entry) };
        loop {
            match next {
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => break,
                Err(error) => return Err(error),
            }
            let end = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if matches_process(browser, &name, own_session, own_session)
                && let Ok(id) = session(entry.th32ProcessID)
                && matches_process(browser, &name, id, own_session)
            {
                candidates.push(entry.th32ProcessID);
            }
            // A process exiting between the snapshot and its session query is ignored, as Get-Process did.
            // SAFETY: same owned handle and live entry as above.
            next = unsafe { Process32NextW(snapshot.0, &raw mut entry) };
        }
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut state = Windows {
            candidates,
            found: false,
        };
        // SAFETY: visit only reads HWND facts and updates this value during the synchronous call.
        unsafe { EnumWindows(Some(visit), LPARAM(&raw mut state as isize)) }?;
        Ok(state.found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_browser_selection_matches_main_window_and_session_rules() {
        assert!(matches_process(Browser::Chrome, "CHROME.EXE", 7, 7));
        assert!(matches_process(Browser::Edge, "msedge.exe", 7, 7));
        assert!(!matches_process(Browser::Chrome, "chrome.exe", 8, 7));
        assert!(!matches_process(Browser::Chrome, "msedge.exe", 7, 7));
        assert!(!matches_process(Browser::Edge, "chrome.exe", 7, 7));
        assert!(!matches_process(Browser::Chrome, "chrome-helper.exe", 7, 7));
        assert!(main_window(42, true, false, &[42]));
        assert!(!main_window(42, false, false, &[42]));
        assert!(!main_window(42, true, true, &[42]));
        assert!(!main_window(41, true, false, &[42]));
        assert!(!main_window(42, true, false, &[]));
    }
}
