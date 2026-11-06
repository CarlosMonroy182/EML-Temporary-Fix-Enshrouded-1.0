use linuxtest::native::{config, patch, spawner};

#[repr(C)]
struct Text {
    ptr: *const u8,
    len: u64,
}

#[test]
fn an_answer_for_one_player_is_taken_back_from_everybody_else() {
    let line = b"[M182#7] 1=Ana|2=Luis";
    let mut queue = vec![0u8; 0x400];
    let mut entry = vec![0u8; 0x80];
    entry[8..12].copy_from_slice(&(line.len() as u32).to_le_bytes());
    entry[0x40..0x40 + line.len()].copy_from_slice(line);
    let size = 44u32; // (21 + 0x17), already a multiple of 4
    let mut mask = 0u32;
    for peer in 0..32usize {
        if peer < 3 {
            queue[0x0c + peer * 12] = 1;
            queue[0x14 + peer * 12..0x14 + peer * 12 + 4].copy_from_slice(&(100u32 + size).to_le_bytes());
        } else {
            mask |= 1 << peer;
        }
    }
    entry[0x20..0x24].copy_from_slice(&mask.to_le_bytes());
    let entry_at = entry.as_ptr() as u64;
    queue[0x1c8..0x1d0].copy_from_slice(&entry_at.to_le_bytes());

    assert!(patch::narrow_last_entry(queue.as_ptr() as usize, 1, line));

    let counter = |p: usize| u32::from_le_bytes(queue[0x14 + p * 12..0x14 + p * 12 + 4].try_into().unwrap());
    assert_eq!((counter(0), counter(1), counter(2)), (100, 100 + size, 100), "only the asker keeps the bytes pending");
    let mask = u32::from_le_bytes(entry[0x20..0x24].try_into().unwrap());
    assert_eq!(mask & 7, 0b101, "everybody but the asker is marked as done");
    assert_eq!(mask >> 3, u32::MAX >> 3, "players that were not connected stay marked");

    // not the entry that was just queued: nothing is touched
    let before = queue.clone();
    assert!(!patch::narrow_last_entry(queue.as_ptr() as usize, 1, b"something else"));
    assert_eq!(queue, before);
}

#[test]
fn a_game_connected_to_a_server_reads_its_answers_from_the_chat_line_and_never_shows_them() {
    let dir = std::env::temp_dir().join(format!("m182_reply_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();
    config::LIVE.apply(&config::NativeConfig { enabled: true, ..Default::default() });

    let say = |s: &str| -> (Vec<u8>, Text) {
        let v = s.as_bytes().to_vec();
        let t = Text { ptr: v.as_ptr(), len: v.len() as u64 };
        (v, t)
    };
    let (_k, t) = say("[M182#42] 1=moradoox|1037=ID 1037 ^1@3206.3,834.0,2059.5");
    assert!(patch::take_server_reply(&t as *const Text as usize));
    let online = std::fs::read_to_string(dir.join(spawner::ONLINE_FILE)).unwrap();
    assert!(online.contains("\"seq\":42") && online.contains("moradoox") && online.contains("\"me\""), "{online}");

    let (_k, t) = say("[M182T#43] Red#Ana,Luis#Ana;Azul##$");
    assert!(patch::take_server_reply(&t as *const Text as usize));
    let teams = std::fs::read_to_string(dir.join("m182_teams_live.json")).unwrap();
    assert!(teams.contains("Red") && teams.contains("Azul"), "{teams}");

    let (_k, t) = say("hola a todos");
    assert!(!patch::take_server_reply(&t as *const Text as usize), "ordinary chat is never taken");
    let (_k, t) = say("[M182] mensaje de alguien que no es una respuesta");
    assert!(!patch::take_server_reply(&t as *const Text as usize));
    let _ = std::fs::remove_dir_all(&dir);
}
