use std::arch::asm;
use linuxtest::native::{config, patch, signatures, table};
use std::sync::atomic::Ordering;

unsafe fn lookup(f: usize, out: *mut u8, entity: usize, pool: usize) -> (u64, bool) {
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
            "mov r12, 0x2222222222222222", "cmp r10, r12", "je 3f", "xor r14, r14", "3:",
            "mov r12, 0x3333333333333333", "cmp r11, r12", "je 4f", "xor r14, r14", "4:",
            "movq r8, xmm0", "mov r12, 0x4444444444444441", "cmp r8, r12", "je 5f", "xor r14, r14", "5:",
            "movq r8, xmm1", "mov r12, 0x4444444444444442", "cmp r8, r12", "je 6f", "xor r14, r14", "6:",
            "movq r8, xmm2", "mov r12, 0x4444444444444443", "cmp r8, r12", "je 7f", "xor r14, r14", "7:",
            "movq r8, xmm3", "mov r12, 0x4444444444444444", "cmp r8, r12", "je 8f", "xor r14, r14", "8:",
            "movq r8, xmm4", "mov r12, 0x4444444444444445", "cmp r8, r12", "je 9f", "xor r14, r14", "9:",
            "movq r8, xmm5", "mov r12, 0x4444444444444446", "cmp r8, r12", "je 20f", "xor r14, r14", "20:",
            in("r13") f, in("rcx") out, in("rdx") entity, in("r8") pool,
            lateout("rax") ret, out("r14") ok,
            out("r9") _, out("r10") _, out("r11") _, out("r12") _, out("r15") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _, out("xmm4") _, out("xmm5") _,
        );
    }
    (ret, ok == 1)
}

fn players(slots: &[(usize, u32, u8, &str)]) -> Vec<u8> {
    let mut t = vec![0u8; 0x1e0 + 17 * 0x148 + 64];
    for (index, client, perms, name) in slots {
        let at = 0x1e0 + index * 0x148;
        t[at..at + 4].copy_from_slice(&client.to_le_bytes());
        t[at + 0x0a] = 3;
        t[at + 0x10] = *perms;
        t[at + 0x20..at + 0x20 + name.len()].copy_from_slice(name.as_bytes());
    }
    t
}

#[test]
fn a_visitor_loses_chests_and_building_where_the_game_checks_them() {
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
        std::ptr::copy_nonoverlapping(signatures::PLAYER_PERMISSIONS.as_ptr(), func, signatures::PLAYER_PERMISSIONS.len());

        config::LIVE.apply(&config::NativeConfig { enabled: true, spawner: true, ..Default::default() });
        assert!(!patch::permissions_hooked());
        assert_eq!(patch::install(), patch::Outcome::Done);
        assert!(patch::permissions_hooked());

        let mut pool = vec![0u8; 0xa0 * 8];
        for entity in 1u32..=3 {
            let at = 0xa0 * (entity as usize - 1);
            pool[at..at + 4].copy_from_slice(&entity.to_le_bytes());
            pool[at + 0x9d] = 0b11_1111;
        }
        let mut out = [0xAAu8; 8];
        let mut ask = |entity: usize| {
            out.fill(0xAA);
            let (ret, ok) = lookup(func as usize, out.as_mut_ptr(), entity, pool.as_ptr() as usize);
            assert!(ok, "registers clobbered");
            assert_eq!(ret, out.as_mut_ptr() as u64, "the lookup still hands back its out pointer");
            out[0]
        };

        assert_eq!(ask(2), 0b11_1111, "nobody is restricted yet");
        patch::restrict_permissions(0b010);
        assert_eq!(ask(1), 0b11_1111);
        assert_eq!(ask(2), 0b10_0001, "entity 2 keeps the base permissions that are not chests or building");
        assert_eq!(ask(3), 0b11_1111);
        patch::restrict_permissions(0);
        assert_eq!(ask(2), 0b11_1111, "leaving the zone gives everything back");

        // the player table drives the mask
        let table_bytes = players(&[(0, 128, 0b11_1111, "Bob"), (1, 65, 0b11_1110, "Ana")]);
        patch::PLAYERS_OBJECT.store(table_bytes.as_ptr() as usize, Ordering::SeqCst);
        table::apply_visitors(&[1]);
        assert_eq!(patch::restricted_permissions(), 0b10);
        assert_eq!(table_bytes[0x1e0 + 0x148 + 0x10], 0b11_1110, "the stored byte is left alone");
        assert_eq!(ask(2), 0b10_0001, "Ana's lookups lose chests and building");
        table::apply_visitors(&[]);
        assert_eq!(patch::restricted_permissions(), 0);
        assert_eq!(ask(2), 0b11_1111);
        patch::PLAYERS_OBJECT.store(0, Ordering::SeqCst);
    }
}
