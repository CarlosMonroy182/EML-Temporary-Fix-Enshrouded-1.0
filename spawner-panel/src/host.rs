//! The in-game window: opened and closed with Insert, and the hands that type commands into the
//! game chat. Everything that touches the operating system goes through [`Sys`] so the logic can
//! be tested without a game.

use std::sync::{Arc, Mutex};

use crate::remote::Typist;

pub const VK_INSERT: i32 = 0x2D;

/// Key that opens and closes the panel; Insert unless `panel_key` in the settings file says otherwise.
pub static TOGGLE_VK: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(VK_INSERT);

/// `F1`..`F12`, `Insert`, `Home`, `End`, `Delete`, `PageUp`, `PageDown` or `Pause` as a Windows key code.
pub fn parse_panel_key(name: &str) -> Option<i32> {
    let t = name.trim().to_ascii_lowercase();
    if let Some(n) = t.strip_prefix('f').and_then(|n| n.parse::<i32>().ok()) {
        return (1..=12).contains(&n).then_some(0x6F + n);
    }
    match t.as_str() {
        "insert" | "ins" => Some(0x2D),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "delete" | "del" => Some(0x2E),
        "pageup" | "pgup" => Some(0x21),
        "pagedown" | "pgdn" => Some(0x22),
        "pause" => Some(0x13),
        _ => None,
    }
}

/// Reads `panel_key` from the settings text and makes it the panel's key.
pub fn apply_panel_key(settings: &str) {
    let key = serde_json::from_str::<serde_json::Value>(settings)
        .ok()
        .and_then(|v| v.get("panel_key").and_then(|k| k.as_str()).and_then(parse_panel_key))
        .unwrap_or(VK_INSERT);
    TOGGLE_VK.store(key, std::sync::atomic::Ordering::SeqCst);
}

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
    /// Key kept down for `ms` milliseconds, so a game that samples its keys once per frame sees it.
    fn hold(&self, scan: u16, _ms: u64) {
        self.tap(scan);
    }
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
        let down = self.sys.key_down(TOGGLE_VK.load(std::sync::atomic::Ordering::SeqCst));
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
    pub window_lines: usize,
    pub window_ms: u64,
    pub hold: u64,
    pub confirm_ms: u64,
}

impl Default for Pace {
    fn default() -> Self {
        Self { burst: 2, gap: 3, opened: 300, before_send: 80, after_send: 120, window_lines: 12, window_ms: 15_000, hold: 120, confirm_ms: 3_000 }
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
        if let Some(n) = get("typing_lines") {
            pace.window_lines = (n as usize).clamp(1, 200);
        }
        if let Some(n) = get("typing_window_s") {
            pace.window_ms = n.clamp(1, 120) * 1000;
        }
        if let Some(n) = get("typing_hold_ms") {
            pace.hold = n.clamp(20, 500);
        }
        pace
    }
}

/// Tells how many chat lines the game has finished sending since the last question.
pub trait Acks: Send {
    fn fresh(&mut self) -> usize;
}

/// The game writes `closeVirtualKeyboard` to its own log every time the chat closes after a message.
pub struct LogAcks {
    path: std::path::PathBuf,
    pos: u64,
}

const ACK_MARK: &str = "closeVirtualKeyboard";

impl LogAcks {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let pos = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Self { path, pos }
    }
}

impl Acks for LogAcks {
    fn fresh(&mut self) -> usize {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut file) = std::fs::File::open(&self.path) else { return 0 };
        let Ok(len) = file.metadata().map(|m| m.len()) else { return 0 };
        if len < self.pos {
            self.pos = 0;
        }
        if len == self.pos || file.seek(SeekFrom::Start(self.pos)).is_err() {
            return 0;
        }
        let mut bytes = Vec::new();
        if file.take((len - self.pos).min(1 << 20)).read_to_end(&mut bytes).is_err() {
            return 0;
        }
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else { return 0 };
        self.pos += end as u64 + 1;
        String::from_utf8_lossy(&bytes[..=end]).matches(ACK_MARK).count()
    }
}

/// Starts the error text of a failure after which the same job must not be typed again.
pub const STOP_MARK: &str = "[stop]";

pub struct ChatTypist<S: Sys> {
    host: Arc<Host<S>>,
    open_scan: u16,
    pace: Pace,
    sent: std::collections::VecDeque<std::time::Instant>,
    acks: Option<Box<dyn Acks>>,
    proven: bool,
    lag: u64,
}

impl<S: Sys> ChatTypist<S> {
    pub fn new(host: Arc<Host<S>>, open_key: &str) -> Self {
        Self { host, open_scan: scan_of(open_key).unwrap_or(ENTER), pace: Pace::default(), sent: std::collections::VecDeque::new(), acks: None, proven: false, lag: 0 }
    }

    pub fn with_pace(mut self, pace: Pace) -> Self {
        self.pace = pace;
        self
    }

    pub fn with_acks(mut self, acks: Box<dyn Acks>) -> Self {
        self.acks = Some(acks);
        self
    }

    /// Waits for the game to say the line left. Before the first confirmation of the session a
    /// missing answer only switches the check off; afterwards it stops the sending.
    fn confirm(&mut self) -> Result<(), String> {
        let Some(acks) = self.acks.as_mut() else { return Ok(()) };
        let mut waited = 0u64;
        loop {
            if acks.fresh() > 0 {
                self.proven = true;
                self.lag = waited.saturating_sub(self.pace.after_send).min(1_500);
                return Ok(());
            }
            if waited >= self.pace.confirm_ms {
                break;
            }
            self.host.sys.sleep(40);
            waited += 40;
        }
        if !self.proven {
            self.acks = None;
            return Ok(());
        }
        Err(format!("{STOP_MARK} {}", "El juego no confirmó el mensaje: el chat no respondió. Envío detenido para no dejar teclas sueltas en el juego. Mira que el chat esté cerrado y vuelve a intentarlo."))
    }

    fn game_has_focus(&self, game: usize) -> bool {
        self.host.sys.foreground() == game
    }

    /// The game drops chat lines sent in a long uninterrupted run: keep to a lines-per-window budget.
    fn throttle(&mut self) {
        let window = std::time::Duration::from_millis(self.pace.window_ms);
        let now = std::time::Instant::now();
        self.sent.retain(|t| now.duration_since(*t) < window);
        if self.sent.len() >= self.pace.window_lines {
            let oldest = self.sent[self.sent.len() - self.pace.window_lines];
            let wait = window.saturating_sub(now.duration_since(oldest));
            self.host.sys.sleep(wait.as_millis() as u64 + 50);
        }
        self.sent.push_back(std::time::Instant::now());
    }
}

impl<S: Sys> Typist for ChatTypist<S> {
    fn begin(&mut self) -> Result<(), String> {
        let game = self.host.game_window();
        if game == 0 {
            return Err("Abre el panel desde dentro del juego antes de enviar.".to_string());
        }
        for attempt in 0..4 {
            self.host.sys.focus(game);
            self.host.sys.sleep(300);
            if self.game_has_focus(game) {
                return Ok(());
            }
            if attempt < 3 {
                self.host.sys.sleep(500);
            }
        }
        Err("No pude pasar el control al juego. Deja el juego visible.".to_string())
    }

    fn line(&mut self, text: &str) -> Result<(), String> {
        let game = self.host.game_window();
        if !self.game_has_focus(game) {
            return Err("El juego perdió el control mientras se escribía. No toques el teclado ni el ratón durante el envío.".to_string());
        }
        self.throttle();
        if let Some(acks) = self.acks.as_mut() {
            acks.fresh();
        }
        let sys = self.host.sys.clone();
        sys.hold(self.open_scan, self.pace.hold + self.lag / 2);
        sys.sleep(self.pace.opened + self.lag);
        let units: Vec<u16> = text.encode_utf16().collect();
        for burst in units.chunks(self.pace.burst) {
            sys.units(burst);
            sys.sleep(self.pace.gap);
        }
        sys.sleep(self.pace.before_send);
        sys.hold(ENTER, 50);
        sys.sleep(self.pace.after_send);
        self.confirm()
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
        fn hold(&self, scan: u16, _ms: u64) {
            self.note(format!("hold {scan:#x}"));
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
    fn panel_key_names() {
        assert_eq!(parse_panel_key("F1"), Some(0x70));
        assert_eq!(parse_panel_key(" f12 "), Some(0x7B));
        assert_eq!(parse_panel_key("Insert"), Some(VK_INSERT));
        assert_eq!(parse_panel_key("PageDown"), Some(0x22));
        assert_eq!(parse_panel_key("F13"), None);
        assert_eq!(parse_panel_key("x"), None);
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
        assert_eq!(log[1], "hold 0x1c");
        let typed: String = log.iter().filter_map(|l| l.strip_prefix("char ")).collect();
        assert_eq!(typed, "/m182 k ñ");
        assert_eq!(log[log.len() - 2], "hold 0x1c");
        assert_eq!(log.last().unwrap(), "focus 200");
    }

    struct Script(Arc<Mutex<Vec<usize>>>);

    impl Acks for Script {
        fn fresh(&mut self) -> usize {
            let mut v = self.0.lock().unwrap();
            if v.is_empty() { 0 } else { v.remove(0) }
        }
    }

    fn ready(sys: &Arc<Mock>) -> Arc<Host<Mock>> {
        let host = Host::new(sys.clone());
        sys.foreground.store(100, Ordering::SeqCst);
        host.poll(&mut || {});
        sys.insert.store(true, Ordering::SeqCst);
        host.poll(&mut || {});
        sys.insert.store(false, Ordering::SeqCst);
        host
    }

    #[test]
    fn a_line_the_game_does_not_confirm_stops_the_typing_once_confirmations_work() {
        let sys = Arc::new(Mock::default());
        let host = ready(&sys);
        // answers: the pre-line drain (0), the first line is confirmed (1), the second drain (0) then silence
        let script = Arc::new(Mutex::new(vec![0, 1, 0]));
        let mut typist = ChatTypist::new(host, "Enter").with_acks(Box::new(Script(script)));
        typist.begin().unwrap();
        typist.line("one").unwrap();
        let err = typist.line("two").unwrap_err();
        assert!(err.starts_with(STOP_MARK), "{err}");
        let typed: String = sys.take().iter().filter_map(|l| l.strip_prefix("char ")).map(str::to_string).collect();
        assert_eq!(typed, "onetwo", "the failing line was typed, nothing after it");
    }

    #[test]
    fn without_any_confirmation_the_check_switches_itself_off() {
        let sys = Arc::new(Mock::default());
        let host = ready(&sys);
        let script = Arc::new(Mutex::new(Vec::new()));
        let mut typist = ChatTypist::new(host, "Enter").with_acks(Box::new(Script(script)));
        typist.begin().unwrap();
        typist.line("one").unwrap();
        typist.line("two").unwrap();
        typist.line("three").unwrap();
    }

    #[test]
    fn the_game_log_tells_how_many_lines_left() {
        let dir = std::env::temp_dir().join(format!("m182_acks_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("enshrouded.log");
        std::fs::write(&path, "[I 00:00:01,000] [input] closeVirtualKeyboard for user 0(1)\n").unwrap();
        let mut acks = LogAcks::new(&path);
        assert_eq!(acks.fresh(), 0, "what was there before does not count");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("[I 00:00:02,000] [input] closeVirtualKeyboard for user 0(1)\n[I 00:00:02,000] [GameUi] toggleHudScreen to InGame\n[I 00:00:03,000] [input] closeVirtualKey");
        std::fs::write(&path, &text).unwrap();
        assert_eq!(acks.fresh(), 1, "a half written line waits");
        text.push_str("board for user 0(1)\n");
        std::fs::write(&path, &text).unwrap();
        assert_eq!(acks.fresh(), 1);
        assert_eq!(acks.fresh(), 0);
        assert_eq!(LogAcks::new(dir.join("missing.log")).fresh(), 0);
        let _ = std::fs::remove_dir_all(dir);
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
        assert!(typist.begin().unwrap_err().contains("panel"));
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
