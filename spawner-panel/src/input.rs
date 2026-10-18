//! Layout of the Windows keyboard input records handed to `SendInput`.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyboardInput {
    pub vk: u16,
    pub scan: u16,
    pub flags: u32,
    pub time: u32,
    pub extra: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Input {
    pub kind: u32,
    pub key: KeyboardInput,
    pub tail: [u8; 8],
}

pub const INPUT_KEYBOARD: u32 = 1;
pub const KEYEVENTF_KEYUP: u32 = 0x0002;
pub const KEYEVENTF_UNICODE: u32 = 0x0004;
pub const KEYEVENTF_SCANCODE: u32 = 0x0008;

fn record(scan: u16, flags: u32) -> Input {
    Input { kind: INPUT_KEYBOARD, key: KeyboardInput { vk: 0, scan, flags, time: 0, extra: 0 }, tail: [0; 8] }
}

/// Press and release of a key given by its hardware scan code.
pub fn scan_tap(scan: u16) -> [Input; 2] {
    [record(scan, KEYEVENTF_SCANCODE), record(scan, KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP)]
}

/// Press and release of one UTF-16 unit typed as text.
pub fn unit_tap(unit: u16) -> [Input; 2] {
    [record(unit, KEYEVENTF_UNICODE), record(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn the_record_has_the_size_windows_expects() {
        assert_eq!(std::mem::size_of::<Input>(), 40);
        assert_eq!(std::mem::size_of::<KeyboardInput>(), 24);
    }

    #[test]
    fn taps_press_then_release() {
        let [down, up] = scan_tap(0x1C);
        assert_eq!((down.key.scan, down.key.flags), (0x1C, 0x8));
        assert_eq!(up.key.flags, 0xA);
        let [down, up] = unit_tap(0xF1);
        assert_eq!((down.key.scan, down.key.vk, down.key.flags), (0xF1, 0, 0x4));
        assert_eq!(up.key.flags, 0x6);
        assert_eq!(down.kind, 1);
    }
}
