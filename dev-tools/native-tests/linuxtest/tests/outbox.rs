use linuxtest::native::{outbox, probe};
use std::sync::Mutex;
use std::time::Duration;

static SENT: Mutex<Vec<(usize, usize, Vec<u8>, u32, u32)>> = Mutex::new(Vec::new());

extern "win64" fn fake_send(out: *mut u8, queue: usize, kind: usize, text: *const [u64; 2], tag1: u32, tag2: u32) -> usize {
    let text = unsafe { *text };
    let bytes = unsafe { std::slice::from_raw_parts(text[0] as *const u8, text[1] as usize) }.to_vec();
    SENT.lock().unwrap().push((queue, kind, bytes, tag1, tag2));
    unsafe { *out = 0 };
    out as usize
}

#[test]
fn lines_left_in_the_out_file_are_queued_through_the_games_chat_function_while_connected() {
    let dir = std::env::temp_dir().join(format!("m182_outbox_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    outbox::poll(&dir);
    assert!(SENT.lock().unwrap().is_empty());

    probe::set_inject(true);
    outbox::set_send_function(fake_send as usize);
    std::fs::write(dir.join(outbox::OUT_FILE), "first line\n\nsecond line\n").unwrap();
    outbox::poll(&dir);
    assert!(!dir.join(outbox::OUT_FILE).exists());

    outbox::pump();
    assert!(SENT.lock().unwrap().is_empty(), "nothing is sent before the game's per-frame sender has run");
    assert_eq!(std::fs::read_to_string(dir.join(outbox::READY_FILE)).unwrap(), "0", "the panel is told it cannot use this yet");

    // the game's per-frame sender object: its queue pointer lives at +0xf8
    let mut object = vec![0u64; 0x20];
    object[0xf8 / 8] = 0x3002c261418;
    outbox::flush_seen(object.as_ptr() as usize);
    assert_eq!(outbox::known_queue(), 0x3002c261418);
    outbox::poll(&dir);
    assert_eq!(std::fs::read_to_string(dir.join(outbox::READY_FILE)).unwrap(), "1", "and that it can once connected");

    outbox::pump();
    outbox::pump();
    assert_eq!(SENT.lock().unwrap().len(), 1, "one line per pause");
    std::thread::sleep(Duration::from_millis(330));
    outbox::flush_seen(object.as_ptr() as usize);
    outbox::pump();
    let sent = SENT.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], (0x3002c261418, 0, b"first line".to_vec(), 0, 0));
    assert_eq!(sent[1].2, b"second line".to_vec());
    std::thread::sleep(Duration::from_millis(330));
    outbox::pump();
    assert_eq!(SENT.lock().unwrap().len(), 2, "nothing left to send");

    // the connection goes away: the sender stops running, so lines wait instead of touching a dead queue
    std::fs::write(dir.join(outbox::OUT_FILE), "third line\n").unwrap();
    outbox::poll(&dir);
    std::thread::sleep(Duration::from_millis(1700));
    outbox::pump();
    assert_eq!(SENT.lock().unwrap().len(), 2, "no send while the sender is silent");
    outbox::flush_seen(object.as_ptr() as usize);
    outbox::pump();
    assert_eq!(SENT.lock().unwrap().len(), 3, "sent again once the connection is back");

    probe::set_inject(false);
    let _ = std::fs::remove_dir_all(&dir);
}
