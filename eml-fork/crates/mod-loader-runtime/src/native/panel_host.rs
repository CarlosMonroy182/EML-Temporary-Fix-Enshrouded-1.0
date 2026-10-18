//! Loads the in-game admin panel (`M182_Panel.dll`, opened with Insert) when it sits next to
//! the game. The panel is a separate library so the native layer stays small and a missing or
//! broken panel can never affect the game or the server.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

pub const LIBRARY: &str = "M182_Panel.dll";
const ENTRY: &[u8] = b"m182_panel_start\0";
const START_DELAY: Duration = Duration::from_secs(6);

fn wide(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

pub fn start(dir: PathBuf) {
    let library = dir.join(LIBRARY);
    if !library.is_file() {
        tracing::info!(target: "m182::native", file = ?library, "panel library not found, in-game panel off");
        return;
    }
    let spawned = std::thread::Builder::new().name("m182-panel-loader".into()).spawn(move || {
        std::thread::sleep(START_DELAY);
        let module = unsafe { LoadLibraryW(wide(&library).as_ptr()) };
        if module.is_null() {
            tracing::warn!(target: "m182::native", file = ?library, "panel library could not be loaded");
            return;
        }
        let entry = unsafe { GetProcAddress(module, ENTRY.as_ptr()) };
        if entry.is_null() {
            tracing::warn!(target: "m182::native", "panel library has no start function");
            return;
        }
        let start: extern "C" fn(*const u16) -> i32 = unsafe { std::mem::transmute(entry) };
        let folder = wide(&dir);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| start(folder.as_ptr())));
        match result {
            Ok(0) => tracing::info!(target: "m182::native", "in-game panel ready (Insert)"),
            Ok(code) => tracing::warn!(target: "m182::native", code, "in-game panel did not start"),
            Err(_) => tracing::warn!(target: "m182::native", "in-game panel failed while starting"),
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(target: "m182::native", error = %e, "could not start panel loader thread");
    }
}
