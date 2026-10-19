//! The in-game window: opened and closed with Insert, and the hands that type commands into the
//! game chat. Everything that touches the operating system goes through [`Sys`] so the logic can
//! be tested without a game.

use std::sync::{Arc, Mutex};

use crate::remote::Typist;

pub const VK_INSERT: i32 = 0x2D;

pub trait Sys: Send + Sync + 'static {
    fn key_down(&self, vk: i32) -> bool;
    /// Window that has the keyboard focus (0 when none).
    fn foreground(&self) -> usize;
    /// True when the window belongs to this process.
    fn is_ours(&self, hwnd: usize) -> bool;
    /// The panel's own window (0 until it exists).
    fn find_panel(&self) -> usize;
    fn show(&self, hwnd: usize);
    fn hide(&self, hwnd: usize);
    fn focus(&self, hwnd: usize) -> bool;
    fn tap(&self, scan: u16);
    fn unit(&self, unit: u16);
    /// Several characters handed to the keyboard queue in one go.
    fn units(&self, units: &[u16]) {
        for u in units {
            self.unit(*u);
        }
    }
    fn sleep(&self, ms: u64);
}

/// Hardware scan code of the key that opens the game's chat.
pub fn scan_of(name: &str) -> Option<u16> {
    match name.trim().to_ascii_lowercase().as_str() {
        "" | "enter" | "return" => Some(0x1C),
        "t" => Some(0x14),
        "y" => Some(0x15),
        "/" | "slash" => Some(0x35),
        _ => None,
    }
}

const ENTER: u16 = 0x1C;

#[derive(Default)]
struct Inner {
    game: usize,
    panel: usize,
    visible: bool,
    created: bool,
    was_down: bool,
}

pub struct Host<S: Sys> {
    sys: Arc<S>,
    inner: Mutex<Inner>,
}

impl<S: Sys> Host<S> {
    pub fn new(sys: Arc<S>) -> Arc<Self> {
        Arc::new(Self { sys, inner: Mutex::new(Inner::default()) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn game_window(&self) -> usize {
        self.lock().game
    }

    pub fn visible(&self) -> bool {
        self.lock().visible
    }

    /// Called every few milliseconds. `create` opens the window the first time Insert is pressed.
    pub fn poll(&self, create: &mut dyn FnMut()) {
        let panel = self.sys.find_panel();
        let foreground = self.sys.foreground();
        let ours = foreground != 0 && self.sys.is_ours(foreground);
        let down = self.sys.key_down(VK_INSERT);
        let mut g = self.lock();
        if panel != 0 {
            g.panel = panel;
        }
        if ours && foreground != g.panel {
            g.game = foreground;
        }
        let pressed = down && !g.was_down;
        g.was_down = down;
        if !pressed || !ours {
            return;
        }
        if !g.created {
            g.created = true;
            g.visible = true;
            drop(g);
            create();
            return;
        }
        if g.visible {
            g.visible = false;
            let (panel, game) = (g.panel, g.game);
            drop(g);
            self.hide_panel(panel, game);
        } else {
            g.visible = true;
            let panel = g.panel;
            drop(g);
            if panel != 0 {
                self.sys.show(panel);
                self.sys.focus(panel);
            }
        }
    }

    fn hide_panel(&self, panel: usize, game: usize) {
        if panel != 0 {
            self.sys.hide(panel);
        }
        if game != 0 {
            self.sys.focus(game);
        }
    }

    /// The window's close button: hide it and give the game its keyboard back.
    pub fn close_requested(&self) {
        let mut g = self.lock();
        g.visible = false;
        let (panel, game) = (g.panel, g.game);
        drop(g);
        self.hide_panel(panel, game);
    }
}

/// Timing of the typed chat lines (milliseconds, except `burst` which counts characters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pace {
    pub burst: usize,
    pub gap: u64,
    pub opened: u64,
    pub before_send: u64,
    pub after_send: u64,
}

impl Default for Pace {
    fn default() -> Self {
        Self { burst: 4, gap: 2, opened: 120, before_send: 60, after_send: 140 }
    }
}

impl Pace {
    /// Defaults overridden by `typing_*` entries of the panel's settings file.
    pub fn from_settings(text: &str) -> Self {
        let mut pace = Self::default();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return pace };
        let get = |key: &str| v.get(key).and_then(serde_json::Value::as_u64);
        if let Some(n) = get("typing_burst") {
            pace.burst = (n as usize).clamp(1, 64);
        }
        if let Some(n) = get("typing_gap_ms") {
            pace.gap = n.min(50);
        }
        pace
    }
}

pub struct ChatTypist<S: Sys> {
    host: Arc<Host<S>>,
    open_scan: u16,
    pace: Pace,
}

impl<S: Sys> ChatTypist<S> {
    pub fn new(host: Arc<Host<S>>, open_key: &str) -> Self {
        Self { host, open_scan: scan_of(open_key).unwrap_or(ENTER), pace: Pace::default() }
    }

    pub fn with_pace(mut self, pace: Pace) -> Self {
        self.pace = pace;
        self
    }

    fn game_has_focus(&self, game: usize) -> bool {
        self.host.sys.foreground() == game
    }
}

impl<S: Sys> Typist for ChatTypist<S> {
    fn begin(&mut self) -> Result<(), String> {
        let sys = &self.host.sys;
        let game = self.host.game_window();
        if game == 0 {
            return Err("Abre el panel con Insert desde dentro del juego antes de enviar.".to_string());
        }
        sys.focus(game);
        sys.sleep(300);
        if !self.game_has_focus(game) {
            return Err("No pude pasar el control al juego. Deja el juego visible y vuelve a intentarlo.".to_string());
        }
        Ok(())
    }

    fn line(&mut self, text: &str) -> Result<(), String> {
        let sys = &self.host.sys;
        let game = self.host.game_window();
        if !self.game_has_focus(game) {
            return Err("El juego perdió el control mientras se escribía. No toques el teclado ni el ratón durante el envío.".to_string());
        }
        sys.tap(self.open_scan);
        sys.sleep(self.pace.opened);
        let units: Vec<u16> = text.encode_utf16().collect();
        for burst in units.chunks(self.pace.burst) {
            sys.units(burst);
            sys.sleep(self.pace.gap);
        }
        sys.sleep(self.pace.before_send);
        sys.tap(ENTER);
        sys.sleep(self.pace.after_send);
        Ok(())
    }

    fn end(&mut self) {
        let (panel, visible) = {
            let g = self.host.lock();
            (g.panel, g.visible)
        };
        if visible && panel != 0 {
            self.host.sys.focus(panel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Mock {
        insert: AtomicBool,
        foreground: AtomicUsize,
        panel: AtomicUsize,
        steals_focus: AtomicBool,
        log: Mutex<Vec<String>>,
    }

    impl Mock {
        fn note(&self, s: impl Into<String>) {
            self.log.lock().unwrap().push(s.into());
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }
    }

    impl Sys for Mock {
        fn key_down(&self, vk: i32) -> bool {
            vk == VK_INSERT && self.insert.load(Ordering::SeqCst)
        }
        fn foreground(&self) -> usize {
            self.foreground.load(Ordering::SeqCst)
        }
        fn is_ours(&self, hwnd: usize) -> bool {
            hwnd == 100 || hwnd == 200
        }
        fn find_panel(&self) -> usize {
            self.panel.load(Ordering::SeqCst)
        }
        fn show(&self, hwnd: usize) {
            self.note(format!("show {hwnd}"));
        }
        fn hide(&self, hwnd: usize) {
            self.note(format!("hide {hwnd}"));
        }
        fn focus(&self, hwnd: usize) -> bool {
            self.note(format!("focus {hwnd}"));
            if !self.steals_focus.load(Ordering::SeqCst) {
                self.foreground.store(hwnd, Ordering::SeqCst);
            }
            true
        }
        fn tap(&self, scan: u16) {
            self.note(format!("tap {scan:#x}"));
        }
        fn unit(&self, unit: u16) {
            self.note(format!("char {}", char::from_u32(unit as u32).unwrap()));
        }
        fn sleep(&self, _ms: u64) {}
    }

    fn press(host: &Host<Mock>, sys: &Mock, created: &mut u32) {
        sys.insert.store(true, Ordering::SeqCst);
        host.poll(&mut || *created += 1);
        sys.insert.store(false, Ordering::SeqCst);
        host.poll(&mut || *created += 1);
    }

    #[test]
    fn insert_creates_then_hides_and_shows_the_window_and_returns_the_keyboard_to_the_game() {
        let sys = Arc::new(Mock::default());
        let host = Host::new(sys.clone());
        let mut created = 0;
        // Insert pressed in another program does nothing
        sys.foreground.store(999, Ordering::SeqCst);
        press(&host, &sys, &mut created);
        assert_eq!(created, 0);
        // in the game: the window is created once
        sys.foreground.store(100, Ordering::SeqCst);
        press(&host, &sys, &mut created);
        assert_eq!(created, 1);
        assert_eq!(host.game_window(), 100);
        assert!(host.visible());
        // the window now exists and has the focus
        sys.panel.store(200, Ordering::SeqCst);
        sys.foreground.store(200, Ordering::SeqCst);
        host.poll(&mut || created += 1);
        assert_eq!(host.game_window(), 100);
        sys.take();
        press(&host, &sys, &mut created);
        assert_eq!(sys.take(), vec!["hide 200", "focus 100"]);
        assert!(!host.visible());
        press(&host, &sys, &mut created);
        assert_eq!(sys.take(), vec!["show 200", "focus 200"]);
        assert_eq!(created, 1);
        // a held key counts once
        sys.insert.store(true, Ordering::SeqCst);
        host.poll(&mut || created += 1);
        host.poll(&mut || created += 1);
        host.poll(&mut || created += 1);
        assert_eq!(sys.take(), vec!["hide 200", "focus 100"]);
        // the close button behaves like Insert
        host.close_requested();
        assert!(!host.visible());
    }

    #[test]
    fn lines_are_typed_into_the_chat_and_the_panel_gets_the_focus_back() {
        let sys = Arc::new(Mock::default());
        let host = Host::new(sys.clone());
        let mut created = 0;
        sys.foreground.store(100, Ordering::SeqCst);
        press(&host, &sys, &mut created);
        sys.panel.store(200, Ordering::SeqCst);
        sys.foreground.store(200, Ordering::SeqCst);
        host.poll(&mut || {});
        sys.take();
        let mut typist = ChatTypist::new(host.clone(), "Enter");
        typist.begin().unwrap();
        typist.line("/m182 k ñ").unwrap();
        typist.end();
        let log = sys.take();
        assert_eq!(log.first().unwrap(), "focus 100");
        assert_eq!(log[1], "tap 0x1c");
        let typed: String = log.iter().filter_map(|l| l.strip_prefix("char ")).collect();
        assert_eq!(typed, "/m182 k ñ");
        assert_eq!(log[log.len() - 2], "tap 0x1c");
        assert_eq!(log.last().unwrap(), "focus 200");
    }

    #[test]
    fn typing_speed_can_be_tuned_from_the_settings_file() {
        assert_eq!(Pace::from_settings("{}"), Pace::default());
        assert_eq!(Pace::from_settings("basura"), Pace::default());
        let p = Pace::from_settings(r#"{"typing_burst":8,"typing_gap_ms":1}"#);
        assert_eq!((p.burst, p.gap), (8, 1));
        let p = Pace::from_settings(r#"{"typing_burst":0,"typing_gap_ms":900}"#);
        assert_eq!((p.burst, p.gap), (1, 50));
    }

    #[test]
    fn typing_stops_when_the_game_does_not_have_the_keyboard() {
        let sys = Arc::new(Mock::default());
        let host = Host::new(sys.clone());
        let mut typist = ChatTypist::new(host.clone(), "t");
        assert!(typist.begin().unwrap_err().contains("Insert"));
        sys.foreground.store(100, Ordering::SeqCst);
        host.poll(&mut || {});
        sys.insert.store(true, Ordering::SeqCst);
        host.poll(&mut || {});
        sys.insert.store(false, Ordering::SeqCst);
        sys.steals_focus.store(true, Ordering::SeqCst);
        sys.foreground.store(777, Ordering::SeqCst);
        assert!(typist.begin().unwrap_err().contains("control"));
        sys.steals_focus.store(false, Ordering::SeqCst);
        sys.foreground.store(100, Ordering::SeqCst);
        typist.begin().unwrap();
        sys.foreground.store(777, Ordering::SeqCst);
        assert!(typist.line("x").unwrap_err().contains("perdió"));
        assert!(!sys.take().iter().any(|l| l.starts_with("char")));
    }

    #[test]
    fn chat_keys() {
        assert_eq!(scan_of("Enter"), Some(0x1C));
        assert_eq!(scan_of(""), Some(0x1C));
        assert_eq!(scan_of("T"), Some(0x14));
        assert_eq!(scan_of("/"), Some(0x35));
        assert_eq!(scan_of("F13"), None);
    }
}
