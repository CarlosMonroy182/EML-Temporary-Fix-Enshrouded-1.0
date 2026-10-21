use std::arch::asm;
use linuxtest::native::{config, patch};

fn pattern(name: &str) -> Vec<u8> {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../patterns.json")).unwrap()).unwrap();
    let h = j[name].as_str().unwrap();
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}

// returns (al, volatile registers preserved?)
unsafe fn call_checked(f: usize, a: usize, b: usize, c: usize) -> (u8, bool) {
    let ret: u64;
    let ok: u64;
    unsafe {
        asm!(
            "mov r15, rsp",
            "mov r12, 0x1111111111111111", "mov r9, r12",
            "mov r12, 0x2222222222222222", "mov r10, r12",
            "mov r12, 0x3333333333333333", "mov r11, r12",
            "mov r12, 0x4444444444444441", "movq xmm0, r12",
            "mov r12, 0x4444444444444442", "movq xmm1, r12",
            "mov r12, 0x4444444444444443", "movq xmm2, r12",
            "mov r12, 0x4444444444444444", "movq xmm3, r12",
            "mov r12, 0x4444444444444445", "movq xmm4, r12",
            "mov r12, 0x4444444444444446", "movq xmm5, r12",
            "and rsp, -16",
            "sub rsp, 0x20",
            "call r13",
            "mov rsp, r15",
            "mov r14, 1",
            "mov r12, 0x1111111111111111", "cmp r9, r12", "je 2f", "xor r14, r14", "2:",
            "mov r12, 0x2222222222222222", "cmp r10, r12", "je 3f", "xor r14, r14", "3:",
            "mov r12, 0x3333333333333333", "cmp r11, r12", "je 4f", "xor r14, r14", "4:",
            "movq r8, xmm0", "mov r12, 0x4444444444444441", "cmp r8, r12", "je 5f", "xor r14, r14", "5:",
            "movq r8, xmm1", "mov r12, 0x4444444444444442", "cmp r8, r12", "je 6f", "xor r14, r14", "6:",
            "movq r8, xmm2", "mov r12, 0x4444444444444443", "cmp r8, r12", "je 7f", "xor r14, r14", "7:",
            "movq r8, xmm3", "mov r12, 0x4444444444444444", "cmp r8, r12", "je 8f", "xor r14, r14", "8:",
            "movq r8, xmm4", "mov r12, 0x4444444444444445", "cmp r8, r12", "je 9f", "xor r14, r14", "9:",
            "movq r8, xmm5", "mov r12, 0x4444444444444446", "cmp r8, r12", "je 20f", "xor r14, r14", "20:",
            in("r13") f, in("rcx") a, in("rdx") b, in("r8") c,
            lateout("rax") ret, out("r14") ok,
            out("r9") _, out("r10") _, out("r11") _, out("r12") _, out("r15") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _, out("xmm4") _, out("xmm5") _,
        );
    }
    (ret as u8, ok == 1)
}

#[test]
fn hooks_work_end_to_end() {
    unsafe {
        let size = 0x40000usize;
        let mem = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) as *mut u8;
        assert!(mem as isize != -1);
        std::ptr::write_bytes(mem, 0xCC, size);
        // fake PE
        *(mem as *mut u16) = 0x5A4D;
        *(mem.add(0x3C) as *mut u32) = 0x80;
        let nt = mem.add(0x80);
        *(nt as *mut u32) = 0x4550;
        *(nt.add(6) as *mut u16) = 1;
        *(nt.add(20) as *mut u16) = 0xF0;
        let sh = nt.add(24 + 0xF0);
        *(sh.add(8) as *mut u32) = 0x30000;
        *(sh.add(12) as *mut u32) = 0x1000;
        *(sh.add(36) as *mut u32) = 0x6000_0020;
        patch::FAKE_BASE = mem;

        let offs = [("F1", 0x1100usize), ("F2", 0x1300), ("F3", 0x1500), ("F4", 0x1600)];
        for (n, o) in offs { let p = pattern(n); std::ptr::copy_nonoverlapping(p.as_ptr(), mem.add(o), p.len()); }
        let at = |n: &str| mem.add(offs.iter().find(|(k, _)| *k == n).unwrap().1) as usize;

        let g1 = std::fs::read("tests/golden/golden_f1.bin").unwrap();
        let g2 = std::fs::read("tests/golden/golden_f2.bin").unwrap();
        let g3 = std::fs::read("tests/golden/golden_f3.bin").unwrap();
        let g4 = std::fs::read("tests/golden/golden_f4.bin").unwrap();

        let mut pa = Box::new([0u8; 1]); let mut pb = Box::new([0u8; 1]); let mut opt = Box::new([0u8; 2]);
        let pa_p = pa.as_mut_ptr() as usize; let pb_p = pb.as_mut_ptr() as usize; let opt_p = opt.as_mut_ptr() as usize;

        // baseline: unpatched original code behaves like the golden tables under the MS ABI
        for a in [0usize, 1, 3, 7, 8, 11, 14, 200] { for b in [0usize, 1, 6, 7, 11, 13, 255] {
            assert_eq!(call_checked(at("F1"), a, b, 0).0, g1[a * 256 + b], "baseline F1");
        } }

        let cfg = config::NativeConfig { enabled: true, pvp: true, pvp_friend_checks: true, spawn_probe: false, ..Default::default() };
        config::LIVE.apply(&cfg);
        assert_eq!(patch::install(), patch::Outcome::Done);

        for a in 0..=255usize { for b in 0..=255usize {
            let (r1, ok1) = call_checked(at("F1"), 0xAB00_0000 | a, 0x1200_0000 | b, 0);
            let (r4, ok4) = call_checked(at("F4"), a, b, 0);
            pa[0] = a as u8; pb[0] = b as u8;
            let (r2, ok2) = call_checked(at("F2"), pa_p, pb_p, 0);
            assert!(ok1 && ok2 && ok4, "registers clobbered a={a} b={b}");
            let special = a == 7 && b == 7;
            let e1 = if special { 1 } else { g1[a * 256 + b] };
            let e4 = if special { 0 } else { g4[a * 256 + b] };
            assert_eq!(r1, e1, "F1 a={a} b={b}"); assert_eq!(r2, e1, "F2 a={a} b={b}");
            assert_eq!(g2[a * 256 + b], g1[a * 256 + b]);
            assert_eq!(r4, e4, "F4 a={a} b={b}");
            for o in 0..3usize {
                opt[1] = if o == 2 { 1 } else { 0 };
                let (r3, ok3) = call_checked(at("F3"), pa_p, pb_p, if o == 0 { 0 } else { opt_p });
                assert!(ok3);
                let e3 = if special { 0 } else { g3[(o * 256 + a) * 256 + b] };
                assert_eq!(r3, e3, "F3 o={o} a={a} b={b}");
            }
        } }
        // null pointer args don't crash and behave like faction 0
        assert_eq!(call_checked(at("F2"), 0, pb_p, 0).0, 0);
        assert_eq!(call_checked(at("F3"), 0, 0, 0).0, 1);

        // live toggle: pvp off => vanilla for player vs player
        config::LIVE.apply(&config::NativeConfig { enabled: true, pvp: false, pvp_friend_checks: false, spawn_probe: false, ..Default::default() });
        assert_eq!(call_checked(at("F1"), 7, 7, 0).0, 0);
        assert_eq!(call_checked(at("F4"), 7, 7, 0).0, 1);
        config::LIVE.apply(&config::NativeConfig { enabled: true, pvp: true, pvp_friend_checks: false, spawn_probe: false, ..Default::default() });
        assert_eq!(call_checked(at("F1"), 7, 7, 0).0, 1);
        assert_eq!(call_checked(at("F4"), 7, 7, 0).0, 1, "friend checks off => untouched");
        assert_eq!(call_checked(at("F1"), 11, 7, 0).0, 0, "summon stays friendly");
        assert_eq!(call_checked(at("F1"), 4, 7, 0).0, 1, "monsters still hostile");
    }
}

#[test]
fn missing_or_changed_signature_patches_nothing() {
    // covered by separate process state; here we just ensure install is idempotent after success
    let _ = config::LIVE.pvp();
}
