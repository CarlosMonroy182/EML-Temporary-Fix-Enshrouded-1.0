//! Runs the panel inside the game process: a window opened with Insert next to the game, and
//! the library entry point the native layer calls.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use crate::host::{ChatTypist, Host};
use crate::{remote, App, WINDOW_TITLE};

#[cfg(windows)]
#[path = "os_windows.rs"]
mod os;

#[cfg(not(windows))]
#[path = "os_other.rs"]
mod os;

pub use os::{Os, RawOs};

static STARTED: AtomicBool = AtomicBool::new(false);
static CONTEXT: Mutex<Option<egui::Context>> = Mutex::new(None);

pub fn wake() {
    if let Some(ctx) = CONTEXT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctx.request_repaint();
    }
}

struct Embedded {
    app: App,
    host: Arc<Host<Os>>,
}

impl eframe::App for Embedded {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.host.close_requested();
        }
        eframe::App::update(&mut self.app, ctx, frame);
    }
}

fn run_window(dir: PathBuf, host: Arc<Host<Os>>) {
    let mut options = crate::native_options();
    options.viewport = options.viewport.with_always_on_top();
    options.event_loop_builder = Some(Box::new(|builder| os::allow_any_thread(builder)));
    let result = eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            *CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(cc.egui_ctx.clone());
            Ok(Box::new(Embedded { app: App::new(cc, dir), host }))
        }),
    );
    if let Err(e) = result {
        let _ = std::fs::write("m182_panel_error.txt", format!("{e}"));
    }
}

fn chat_key(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join(remote::SETTINGS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("chat_open_key").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// 0 = started, 1 = already running.
pub fn start(dir: PathBuf) -> i32 {
    if STARTED.swap(true, Ordering::SeqCst) {
        return 1;
    }
    let host = Host::new(Arc::new(Os));
    remote::install_typist(Box::new(ChatTypist::new(host.clone(), &chat_key(&dir))));
    let guard = crate::rawguard::RawGuard::new(RawOs);
    let spawned = std::thread::Builder::new().name("m182-panel-keys".into()).spawn(move || {
        let mut ticks = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(30));
            ticks = ticks.wrapping_add(1);
            if ticks % 3 == 0 && guard.tick() {
                remote::note("the game's mouse and keyboard were handed back");
            }
            host.poll(&mut || {
                let (dir, host) = (dir.clone(), host.clone());
                let _ = std::thread::Builder::new().name("m182-panel-ui".into()).stack_size(32 * 1024 * 1024).spawn(move || run_window(dir, host));
            });
        }
    });
    if spawned.is_err() {
        STARTED.store(false, Ordering::SeqCst);
        return 3;
    }
    0
}

#[no_mangle]
pub extern "C" fn m182_panel_start(dir: *const u16) -> i32 {
    if dir.is_null() {
        return 2;
    }
    let mut len = 0usize;
    // SAFETY: the caller passes a NUL-terminated UTF-16 path; the scan is capped.
    unsafe {
        while *dir.add(len) != 0 {
            len += 1;
            if len > 32_768 {
                return 2;
            }
        }
    }
    let units = unsafe { std::slice::from_raw_parts(dir, len) };
    let path = PathBuf::from(String::from_utf16_lossy(units));
    std::panic::catch_unwind(move || start(path)).unwrap_or(3)
}
