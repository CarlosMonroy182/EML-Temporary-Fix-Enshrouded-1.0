use linuxtest::native::{config, patch};

#[inline(never)]
fn call_it(f: extern "win64" fn(usize, u32, *const u8, usize) -> usize, d: *const u8) -> usize {
    let r = f(0x1234_5678, 0x7, d, 0);
    std::hint::black_box(r)
}

#[test]
fn probe_logs_spawn_and_runs_original() {
    unsafe {
        let size = 0x40000usize;
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
        *(sh.add(8) as *mut u32) = 0x30000;
        *(sh.add(12) as *mut u32) = 0x1000;
        *(sh.add(36) as *mut u32) = 0x6000_0020;
        patch::FAKE_BASE = mem;

        let sig = linuxtest::native::sig_spawn();
        let f = mem.add(0x2000);
        std::ptr::copy_nonoverlapping(sig.as_ptr(), f, sig.len());
        let tail: [u8; 14] = [0x31, 0xc0, 0x48, 0x81, 0xc4, 0xd8, 0x0e, 0x00, 0x00, 0x41, 0x5e, 0x41, 0x5c, 0x5e];
        let tail2: [u8; 2] = [0x5d, 0xc3];
        std::ptr::copy_nonoverlapping(tail.as_ptr(), f.add(sig.len()), tail.len());
        std::ptr::copy_nonoverlapping(tail2.as_ptr(), f.add(sig.len() + tail.len()), 2);

        let mut comp_a = [0xAAu8; 44];
        comp_a[0] = 1;
        let comp_b = [7u8; 1];
        let mut tr = [0u8; 56];
        tr[0..8].copy_from_slice(&(5i64 << 32).to_le_bytes());
        tr[8..16].copy_from_slice(&(-3i64).to_le_bytes());
        tr[16..24].copy_from_slice(&(7i64 << 32).to_le_bytes());
        let lvl = 42u32.to_le_bytes();
        let mut entries = [0u8; 96];
        entries[0..4].copy_from_slice(&0xfcee256bu32.to_le_bytes());
        entries[8..16].copy_from_slice(&(comp_a.as_ptr() as u64).to_le_bytes());
        entries[16..24].copy_from_slice(&44u64.to_le_bytes());
        entries[24..28].copy_from_slice(&0x028f11fbu32.to_le_bytes());
        entries[32..40].copy_from_slice(&(comp_b.as_ptr() as u64).to_le_bytes());
        entries[40..48].copy_from_slice(&1u64.to_le_bytes());
        entries[48..52].copy_from_slice(&0x027ac564u32.to_le_bytes());
        entries[56..64].copy_from_slice(&(tr.as_ptr() as u64).to_le_bytes());
        entries[64..72].copy_from_slice(&56u64.to_le_bytes());
        entries[72..76].copy_from_slice(&0xa21e7ee7u32.to_le_bytes());
        entries[80..88].copy_from_slice(&(lvl.as_ptr() as u64).to_le_bytes());
        entries[88..96].copy_from_slice(&4u64.to_le_bytes());
        let mut desc = [0u8; 0x30];
        desc[4..20].copy_from_slice(&[0x11u8; 16]);
        desc[0x18..0x20].copy_from_slice(&(entries.as_ptr() as u64).to_le_bytes());
        desc[0x20..0x28].copy_from_slice(&4u64.to_le_bytes());

        config::LIVE.apply(&config::NativeConfig { spawn_probe: true, ..Default::default() });
        assert_eq!(patch::install(), patch::Outcome::Done);

        let call: extern "win64" fn(usize, u32, *const u8, usize) -> usize = std::mem::transmute(f);
        for _ in 0..8 {
            let r = call_it(call, desc.as_ptr());
            assert_eq!(r & 0xffff_ffff, 0);
        }
        let log = std::fs::read_to_string("m182_spawn_probe.log").unwrap();
        println!("{log}");
        assert!(log.contains("hash=fcee256b size=44"));
        assert!(log.contains("hash=028f11fb size=1 data=07"));
        assert!(log.contains("edx=0x7 "));
        assert!(log.contains("seen=6"));
        assert!(!log.contains("seen=7"));
        let csv = std::fs::read_to_string("m182_spawn_positions.csv").unwrap();
        println!("{csv}");
        assert!(csv.starts_with("t_ms,caller,mgr,edx,hint,guid,level,x,y,z"));
        assert!(csv.lines().count() == 9);
        assert!(csv.lines().nth(1).unwrap().ends_with("00000007,00000000,11111111111111111111111111111111,42,21474836480,-3,30064771072") || csv.lines().nth(1).unwrap().contains(",42,21474836480,-3,30064771072"));
        assert!(log.matches("c0 hash").count() == 6);
    }
}
