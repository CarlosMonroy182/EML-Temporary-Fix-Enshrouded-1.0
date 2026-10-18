//! Stand-in used where there is no Windows game to drive (tests, previews).

use crate::host::Sys;

pub struct Os;

pub fn allow_any_thread<T: 'static>(_builder: &mut winit::event_loop::EventLoopBuilder<T>) {}

impl Sys for Os {
    fn key_down(&self, _vk: i32) -> bool {
        false
    }
    fn foreground(&self) -> usize {
        0
    }
    fn is_ours(&self, _hwnd: usize) -> bool {
        false
    }
    fn find_panel(&self) -> usize {
        0
    }
    fn show(&self, _hwnd: usize) {}
    fn hide(&self, _hwnd: usize) {}
    fn focus(&self, _hwnd: usize) -> bool {
        false
    }
    fn tap(&self, _scan: u16) {}
    fn unit(&self, _unit: u16) {}
    fn sleep(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
