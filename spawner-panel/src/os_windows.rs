//! Windows side of the in-game panel: keyboard state, window handling and key injection.

use std::ptr::null;

use crate::host::Sys;
use crate::input::{scan_tap, unit_tap, Input};
use crate::WINDOW_TITLE;

#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetForegroundWindow() -> usize;
    fn GetWindowThreadProcessId(hwnd: usize, pid: *mut u32) -> u32;
    fn FindWindowW(class: *const u16, title: *const u16) -> usize;
    fn ShowWindow(hwnd: usize, command: i32) -> i32;
    fn SetForegroundWindow(hwnd: usize) -> i32;
    fn BringWindowToTop(hwnd: usize) -> i32;
    fn IsIconic(hwnd: usize) -> i32;
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn AttachThreadInput(from: u32, to: u32, attach: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
}

const SW_HIDE: i32 = 0;
const SW_SHOW: i32 = 5;
const SW_RESTORE: i32 = 9;

pub struct Os;

pub fn allow_any_thread<T: 'static>(builder: &mut winit::event_loop::EventLoopBuilder<T>) {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    builder.with_any_thread(true);
}

fn send(inputs: &[Input]) {
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<Input>() as i32);
    }
}

impl Sys for Os {
    fn key_down(&self, vk: i32) -> bool {
        (unsafe { GetAsyncKeyState(vk) } as u16) & 0x8000 != 0
    }

    fn foreground(&self) -> usize {
        unsafe { GetForegroundWindow() }
    }

    fn is_ours(&self, hwnd: usize) -> bool {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            pid != 0 && pid == GetCurrentProcessId()
        }
    }

    fn find_panel(&self) -> usize {
        let title: Vec<u16> = WINDOW_TITLE.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = unsafe { FindWindowW(null(), title.as_ptr()) };
        if hwnd != 0 && self.is_ours(hwnd) {
            hwnd
        } else {
            0
        }
    }

    fn show(&self, hwnd: usize) {
        unsafe {
            ShowWindow(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
        }
        crate::embed::wake();
    }

    fn hide(&self, hwnd: usize) {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
    }

    fn focus(&self, hwnd: usize) -> bool {
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            let current = GetForegroundWindow();
            let this = GetCurrentThreadId();
            let other = if current != 0 { GetWindowThreadProcessId(current, std::ptr::null_mut()) } else { 0 };
            let attached = other != 0 && other != this && AttachThreadInput(this, other, 1) != 0;
            BringWindowToTop(hwnd);
            let done = SetForegroundWindow(hwnd) != 0;
            if attached {
                AttachThreadInput(this, other, 0);
            }
            done
        }
    }

    fn tap(&self, scan: u16) {
        send(&scan_tap(scan));
    }

    fn unit(&self, unit: u16) {
        send(&unit_tap(unit));
    }

    fn sleep(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
