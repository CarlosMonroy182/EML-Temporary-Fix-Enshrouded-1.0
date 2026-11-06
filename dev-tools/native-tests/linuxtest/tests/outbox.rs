use linuxtest::native::{outbox, patch, probe};
use std::sync::Mutex;
use std::time::Duration;

static SENT: Mutex<Vec<(usize, usize, Vec<u8>, u32, u32, u32, u64, u64)>> = Mutex::new(Vec::new());

extern "win64" fn fake_enqueue(out: *mut u8, queue: usize, kind: usize, text: *const [u64; 2], a5: u32, a6: u32, a7: u32, a8: u64, optional: *const [u64; 2]) -> usize {
    let text = unsafe { *text };
    let bytes = unsafe { std::slice::from_raw_parts(text[0] as *const u8, text[1] as usize) }.to_vec();
    let flag = unsafe { (*optional)[0] };
    SENT.lock().unwrap().push((queue, kind, bytes, a5, a6, a7, a8, flag));
    unsafe { *out = 0 };
    out as usize
}

#[test]
fn lines_left_in_the_out_file_go_through_the_games_chat_call_with_the_arguments_the_game_used() {
    let dir = std::env::temp_dir().join(format!("m182_outbox_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    patch::CHAT_TRAMPOLINE.store(fake_enqueue as usize, std::sync::atomic::Ordering::SeqCst);

    outbox::poll(&dir);
    assert!(SENT.lock().unwrap().is_empty());

    probe::set_inject(true);
    std::fs::write(dir.join(outbox::OUT_FILE), "first line\n\nsecond line\n").unwrap();
    outbox::poll(&dir);
    assert!(!dir.join(outbox::OUT_FILE).exists());

    outbox::pump();
    assert!(SENT.lock().unwrap().is_empty(), "nothing is sent before the game has shown how it calls the function");

    let none: [u64; 2] = [0, 0];
    let mut frame = vec![0u64; 12];
    frame[5] = 0x11;
    frame[6] = 0x482;
    frame[7] = 0x33;
    frame[8] = 0x4444_5555_6666;
    frame[9] = &none as *const _ as u64;
    outbox::capture(0x3002c261418, 0, frame.as_ptr() as usize, 4, 0x482);

    outbox::pump();
    outbox::pump();
    assert_eq!(SENT.lock().unwrap().len(), 1, "one line per pause");
    std::thread::sleep(Duration::from_millis(330));
    outbox::pump();
    let sent = SENT.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], (0x3002c261418, 0, b"first line".to_vec(), 0x11, 0x482, 0x33, 0x4444_5555_6666, 0));
    assert_eq!(sent[1].2, b"second line".to_vec());
    std::thread::sleep(Duration::from_millis(330));
    outbox::pump();
    assert_eq!(SENT.lock().unwrap().len(), 2, "nothing left to send");
    probe::set_inject(false);
    let _ = std::fs::remove_dir_all(&dir);
}
