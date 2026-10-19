//! Windows side of the in-game panel: keyboard state, window handling and key injection.

use std::ptr::null;

use crate::host::Sys;
use crate::input::{scan_down, scan_tap, scan_up, unit_tap, Input};
use crate::rawguard::{Raw, RawSys};
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
    fn VkKeyScanW(ch: u16) -> i16;
    fn MapVirtualKeyW(code: u32, kind: u32) -> u32;
    fn GetRegisteredRawInputDevices(devices: *mut RawDevice, count: *mut u32, size: u32) -> u32;
    fn RegisterRawInputDevices(devices: *const RawDevice, count: u32, size: u32) -> i32;
    fn GetClassNameW(hwnd: usize, buffer: *mut u16, max: i32) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawDevice {
    page: u16,
    usage: u16,
    flags: u32,
    target: usize,
}

pub struct RawOs;

impl RawSys for RawOs {
    fn registered(&self) -> Vec<Raw> {
        let size = std::mem::size_of::<RawDevice>() as u32;
        let mut count = 0u32;
        unsafe {
            GetRegisteredRawInputDevices(std::ptr::null_mut(), &mut count, size);
        }
        if count == 0 || count > 64 {
            return Vec::new();
        }
        let mut buffer = vec![RawDevice { page: 0, usage: 0, flags: 0, target: 0 }; count as usize];
        let got = unsafe { GetRegisteredRawInputDevices(buffer.as_mut_ptr(), &mut count, size) };
        if got == u32::MAX {
            return Vec::new();
        }
        buffer.iter().take(got as usize).map(|d| Raw { page: d.page, usage: d.usage, flags: d.flags, target: d.target }).collect()
    }

    fn is_foreign(&self, target: usize) -> bool {
        let mut name = [0u16; 128];
        let len = unsafe { GetClassNameW(target, name.as_mut_ptr(), name.len() as i32) };
        len > 0 && String::from_utf16_lossy(&name[..len as usize]).starts_with("Winit")
    }

    fn register(&self, devices: &[Raw]) -> bool {
        let list: Vec<RawDevice> = devices.iter().map(|d| RawDevice { page: d.page, usage: d.usage, flags: d.flags, target: d.target }).collect();
        unsafe { RegisterRawInputDevices(list.as_ptr(), list.len() as u32, std::mem::size_of::<RawDevice>() as u32) != 0 }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
}

const SHIFT_SCAN: u16 = 0x2A;
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

fn keys_for(unit: u16, out: &mut Vec<Input>) {
    if unit < 0x80 {
        let found = unsafe { VkKeyScanW(unit) };
        if found != -1 {
            let (vk, state) = ((found & 0xFF) as u32, ((found >> 8) & 0xFF) as u8);
            let scan = unsafe { MapVirtualKeyW(vk, 0) } as u16;
            if scan != 0 && state & !1 == 0 {
                if state & 1 == 1 {
                    out.push(scan_down(SHIFT_SCAN));
                }
                out.extend_from_slice(&scan_tap(scan));
                if state & 1 == 1 {
                    out.push(scan_up(SHIFT_SCAN));
                }
                return;
            }
        }
    }
    out.extend_from_slice(&unit_tap(unit));
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

    fn units(&self, units: &[u16]) {
        let mut batch: Vec<Input> = Vec::with_capacity(units.len() * 4);
        for unit in units {
            keys_for(*unit, &mut batch);
        }
        send(&batch);
    }

    fn unit(&self, unit: u16) {
        let mut batch = Vec::with_capacity(4);
        keys_for(unit, &mut batch);
        send(&batch);
    }

    fn sleep(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
