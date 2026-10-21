use std::arch::asm;
use linuxtest::native::{config, patch, signatures};

const TAIL: [u8; 51] = [
    0x48, 0x89, 0xd0, 0x48, 0x01, 0xc8, 0x4c, 0x01, 0xc0,
    0x8b, 0x8c, 0x24, 0x90, 0x00, 0x00, 0x00,
    0x48, 0xc1, 0xe1, 0x28, 0x48, 0x01, 0xc8,
    0x48, 0x8b, 0x5c, 0x24, 0x60, 0x48, 0x8b, 0x7c, 0x24, 0x68, 0x4c, 0x8b, 0x74, 0x24, 0x70,
    0x48, 0x83, 0xc4, 0x30, 0x41, 0x5f, 0x41, 0x5d, 0x41, 0x5c, 0x5e, 0x5d, 0xc3,
];

#[repr(C)]
struct Text {
    ptr: *const u8,
    len: u64,
}

unsafe fn call_chat(f: usize, out: *mut u8, ctx: usize, kind: usize, text: *const Text, client: u32) -> (u64, bool) {
    let ret: u64;
    let ok: u64;
    unsafe {
        asm!(
            "mov r15, rsp",
            "mov r10, 0x5151515151515151",
            "mov r11, 0x6161616161616161",
            "mov rsi, 0x7171717171717171",
            "mov rdi, 0x8181818181818181",
            "and rsp, -16",
            "sub rsp, 0x50",
            "mov dword ptr [rsp + 0x30], r12d",
            "call r13",
            "mov rsp, r15",
            "mov r14, 1",
            "mov r12, 0x5151515151515151", "cmp r10, r12", "je 2f", "xor r14, r14", "2:",
            "mov r12, 0x6161616161616161", "cmp r11, r12", "je 3f", "xor r14, r14", "3:",
            "mov r12, 0x7171717171717171", "cmp rsi, r12", "je 4f", "xor r14, r14", "4:",
            "mov r12, 0x8181818181818181", "cmp rdi, r12", "je 5f", "xor r14, r14", "5:",
            in("r13") f, in("rcx") out, in("rdx") ctx, in("r8") kind, in("r9") text, in("r12") client as u64,
            lateout("rax") ret, out("r14") ok,
            out("r10") _, out("r11") _, out("r15") _, out("rsi") _, out("rdi") _,
            clobber_abi("win64"),
        );
    }
    (ret, ok == 1)
}

#[test]
fn chat_hook_swallows_only_valid_commands() {
    let dir = std::env::temp_dir().join(format!("m182_chat_hook_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();

    unsafe {
        let size = 0x10000usize;
        let mem = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) as *mut u8;
        assert!(mem as isize != -1);
        std::ptr::write_bytes(mem, 0xCC, size);
        *(mem as *mut u16) = 0x5A4D;
        *(mem.add(0x3C) as *mut u32) = 0x80;
        let nt = mem.add(0x80);
        *(nt as *mut u32) = 0x4550;
        *(nt.add(6) as *mut u16) = 1;
        *(nt.add(20) as *mut u16) = 0xF0;
        let sh = nt.add(24 + 0xF0);
        *(sh.add(8) as *mut u32) = 0x8000;
        *(sh.add(12) as *mut u32) = 0x1000;
        *(sh.add(36) as *mut u32) = 0x6000_0020;
        patch::FAKE_BASE = mem;

        let func = mem.add(0x2000);
        std::ptr::copy_nonoverlapping(signatures::CHAT_ENQUEUE.as_ptr(), func, signatures::CHAT_ENQUEUE.len());
        std::ptr::copy_nonoverlapping(TAIL.as_ptr(), func.add(signatures::CHAT_ENQUEUE.len()), TAIL.len());

        let cfg = config::NativeConfig { enabled: true, chat_key: "secret-key".into(), ..Default::default() };
        config::LIVE.apply(&cfg);
        assert_eq!(patch::install(), patch::Outcome::Done);
        assert_ne!(patch::CHAT_TRAMPOLINE.load(std::sync::atomic::Ordering::SeqCst), 0);

        let f = func as usize;
        let mut out = [0xAAu8; 16];
        let (ctx, kind, client) = (0x1234_5000usize, 3usize, 0x2bu32);
        let expected = |out: *mut u8| (ctx as u64).wrapping_add(out as u64).wrapping_add(kind as u64).wrapping_add((client as u64) << 40);

        let say = |s: &str| -> (Vec<u8>, Text) {
            let v = s.as_bytes().to_vec();
            let t = Text { ptr: v.as_ptr(), len: v.len() as u64 };
            (v, t)
        };

        let (_keep, t) = say("hola a todos");
        out.fill(0xAA);
        let (ret, ok) = call_chat(f, out.as_mut_ptr(), ctx, kind, &t, client);
        assert!(ok, "volatile or callee-saved registers were clobbered");
        assert_eq!(ret, expected(out.as_mut_ptr()), "ordinary chat must reach the original function");
        assert_eq!(out, [0xAAu8; 16]);

        let (_keep, t) = say("/m182 secret-key set m182_zones.json {\"zones\": []}");
        out.fill(0xAA);
        let (ret, ok) = call_chat(f, out.as_mut_ptr(), ctx, kind, &t, client);
        assert!(ok);
        assert_eq!(ret, out.as_mut_ptr() as u64, "swallowed message returns the out buffer");
        assert_eq!(out[0], 0);
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
        assert_eq!(out[8], 0xAA);
        assert_eq!(std::fs::read_to_string(dir.join("m182_zones.json")).unwrap(), "{\"zones\": []}");

        let (_keep, t) = say("/m182 wrong-key set m182_teams.json {}");
        out.fill(0xAA);
        let (ret, ok) = call_chat(f, out.as_mut_ptr(), ctx, kind, &t, client);
        assert!(ok);
        assert_eq!(ret, out.as_mut_ptr() as u64);
        assert!(!dir.join("m182_teams.json").exists());

        let long = format!("/m182 secret-key set m182_teams.json {}", "x".repeat(1100));
        let (_keep, t) = say(&long);
        out.fill(0xAA);
        let (ret, ok) = call_chat(f, out.as_mut_ptr(), ctx, kind, &t, client);
        assert!(ok);
        assert_eq!(ret, out.as_mut_ptr() as u64, "oversized messages get the game's own rejection");
        assert_eq!(out[0], 0x12, "oversized messages are never touched");
        assert!(!dir.join("m182_teams.json").exists());

        let empty = Text { ptr: std::ptr::null(), len: 0 };
        let (ret, ok) = call_chat(f, out.as_mut_ptr(), ctx, kind, &empty, client);
        assert!(ok);
        assert_eq!(ret, expected(out.as_mut_ptr()));

        config::LIVE.apply(&config::NativeConfig { enabled: true, ..Default::default() });
        let (_keep, t) = say("/m182 secret-key set m182_powers.json {}");
        let (ret, ok) = call_chat(f, out.as_mut_ptr(), ctx, kind, &t, client);
        assert!(ok);
        assert_eq!(ret, expected(out.as_mut_ptr()), "without a configured key the chat is untouched");
        assert!(!dir.join("m182_powers.json").exists());
    }
}
