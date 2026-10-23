import re,shutil
import os
S=os.path.join(os.path.dirname(os.path.abspath(__file__)),'..','..','eml-fork','crates','mod-loader-runtime','src','native')+'/'
D='/home/claude/m182_native_work/linuxtest/src/native/'
def rd(f): return open(S+f).read()
def wr(f,t): open(D+f,'w').write(t)
for f in ['faction.rs','signatures.rs','config.rs','groups.rs','invites.rs','market.rs','chat.rs','roster.rs']:
    wr(f,rd(f))
wr('signatures.rs',rd('signatures.rs'))
m=rd('mod.rs').replace('#[cfg(windows)]','#[cfg(all())]').replace('#[cfg(not(windows))]','#[cfg(any())]').replace('\nmod patch;','\npub mod patch;').replace('\nmod signatures;','\npub mod signatures;').replace('\nmod spawn_probe;','\npub mod spawn_probe;').replace('\nmod spawner;','\npub mod spawner;').replace('\nmod table;','\npub mod table;').replace('\nmod vfx;','\npub mod vfx;')
m+='\npub fn sig_spawn() -> &\'static [u8] { &signatures::SPAWN_CREATE }\n'
wr('mod.rs',m)
wr('table.rs',rd('table.rs'))
wr('panel_host.rs','use std::path::PathBuf;\npub fn start(_dir: PathBuf) {}\n')
ffi=re.compile(r'#\[link\(name = "(kernel32|user32|winmm)"\)\]\nunsafe extern "system" \{.*?\n\}\n',re.S)
p=rd('patch.rs')
shim='''pub static mut FAKE_BASE: *mut u8 = std::ptr::null_mut();
unsafe fn GetModuleHandleW(_n: *const u16) -> *mut c_void { unsafe { FAKE_BASE.cast() } }
unsafe fn GetCurrentProcess() -> *mut c_void { 1 as *mut c_void }
unsafe fn VirtualProtect(_a: *mut c_void, _s: usize, _n: u32, old: *mut u32) -> i32 { unsafe { *old = 0x20 }; 1 }
unsafe fn FlushInstructionCache(_p: *mut c_void, _a: *const c_void, _s: usize) -> i32 { 1 }
unsafe fn VirtualAlloc(_a: *mut c_void, size: usize, _k: u32, _p: u32) -> *mut c_void { unsafe { libc::mmap(std::ptr::null_mut(), size, 7, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) } }
'''
p=ffi.sub(lambda m:shim,p,count=1)
p=p.replace('unsafe extern "win64" fn stub_pump()','pub unsafe extern "win64" fn stub_pump()').replace('pub(super) static PUMP_TRAMPOLINE','pub static PUMP_TRAMPOLINE')
p=p.replace('pub(super) static CHAT_TRAMPOLINE','pub static CHAT_TRAMPOLINE').replace('pub(super) static PLAYERS_OBJECT','pub static PLAYERS_OBJECT')
wr('patch.rs',p)
sp=rd('spawn_probe.rs')
shim2='''unsafe fn GetModuleHandleW(_n: *const u16) -> *mut c_void { unsafe { super::patch::FAKE_BASE.cast() } }
unsafe fn GetCurrentProcess() -> *mut c_void { 1 as *mut c_void }
unsafe fn VirtualAlloc(_a: *mut c_void, size: usize, _k: u32, _p: u32) -> *mut c_void { unsafe { libc::mmap(std::ptr::null_mut(), size, 7, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) } }
unsafe fn ReadProcessMemory(_p: *mut c_void, addr: *const c_void, buf: *mut c_void, size: usize, read: *mut usize) -> i32 {
    unsafe {
        let local = libc::iovec { iov_base: buf, iov_len: size };
        let remote = libc::iovec { iov_base: addr as *mut c_void, iov_len: size };
        let n = libc::process_vm_readv(libc::getpid(), &local, 1, &remote, 1, 0);
        if n < 0 || (n as usize) < size {
            *read = if n < 0 { 0 } else { n as usize };
            return 0;
        }
        *read = size;
    }
    1
}
'''
sp=ffi.sub(lambda m:shim2,sp,count=1)
sp+='\npub fn set_trampoline(v: usize) { unsafe { M182_SPAWN_TRAMPOLINE = v } }\n'
wr('spawn_probe.rs',sp)
s3=rd('spawner.rs')
shim3='''pub static KEY_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
unsafe fn GetAsyncKeyState(_vk: i32) -> i16 { if KEY_DOWN.load(Ordering::SeqCst) { 0x8000u16 as i16 } else { 0 } }
unsafe fn GetForegroundWindow() -> *mut c_void { 1 as *mut c_void }
unsafe fn GetWindowThreadProcessId(_h: *mut c_void, pid: *mut u32) -> u32 { unsafe { *pid = 4242 }; 0 }
unsafe fn GetCurrentProcessId() -> u32 { 4242 }
unsafe fn timeBeginPeriod(_p: u32) -> u32 { 0 }
unsafe fn VirtualQuery(_a: *const c_void, _i: *mut MemInfo, _s: usize) -> usize { 0 }
unsafe fn VirtualProtect(_a: *mut c_void, _s: usize, _n: u32, old: *mut u32) -> i32 { unsafe { *old = 4 }; 1 }
unsafe fn GetCurrentProcess() -> *mut c_void { 1 as *mut c_void }
unsafe fn WriteProcessMemory(_p: *mut c_void, addr: *mut c_void, buf: *const c_void, size: usize, written: *mut usize) -> i32 { unsafe { std::ptr::copy_nonoverlapping(buf as *const u8, addr as *mut u8, size); *written = size; } 1 }
'''
first=True
def sub3(m):
    global first
    if first:
        first=False
        return shim3
    return ''
s3=ffi.sub(sub3,s3)
# expose internals for tests
for name in ['entity_layout','apply_stats','dump_entity','snapshot','find_record','placement','component_address','calibrate','player_transform','spawn_now','hash_slot','write_bytes','toggle_watch','poll_request','census','entity_ids','write_position','facing_update','facing_direction','refill_health','refill_stamina','refill_mana','set_player_position','power_tick','new_power_state','fly_step','record_trail','run_command','refresh_powers','find_gravity','scan_regions','patch_item_stats','patch_item_struct_by_id','readable_regions','read_window','hex_to_bytes','set_gravity','client_pair','set_client_position','near','track_player','groups_tick','scan_request','roster_line','refresh_client_powers','apply_client_powers','world_pair','remote_world','guess_client_transform','transform_of','is_client_manager','find_roster']:
    s3=s3.replace('\nfn '+name+'(','\npub fn '+name+'(')
s3=s3.replace('pub(super) ','pub ')
s3=s3.replace('\nstruct Placement','\npub struct Placement').replace('    def: usize,\n    chunk: usize,\n    slot: usize,','    pub def: usize,\n    pub chunk: usize,\n    pub slot: usize,')
s3=s3.replace('static PLAYER_ID','pub static PLAYER_ID').replace('static SERVER_MGR','pub static SERVER_MGR').replace('static TRANSFORM_INDEX','pub static TRANSFORM_INDEX').replace('static CLIENT_MGR','pub static CLIENT_MGR').replace('static CLIENT_PLAYER_ID','pub static CLIENT_PLAYER_ID').replace('static CLIENT_TRANSFORM_INDEX','pub static CLIENT_TRANSFORM_INDEX').replace('static PENDING','pub static PENDING').replace('\nstatic REQUEST:','\npub static REQUEST:').replace('static TRACKED_MGR','pub static TRACKED_MGR').replace('\nstruct GroupState','\npub struct GroupState').replace('\nstruct Tracked','\npub struct Tracked')
import re as _re
blk=_re.search(r'\nstruct PowerState \{.*?\n\}',s3,_re.S).group(0)
s3=s3.replace(blk,_re.sub(r'\n    (\w+):',r'\n    pub \1:',blk).replace('\nstruct','\npub struct'))
blk=_re.search(r'\nstruct Powers \{.*?\n\}',s3,_re.S).group(0)
s3=s3.replace(blk,_re.sub(r'\n    (\w+):',r'\n    pub \1:',blk).replace('\nstruct','\npub struct'))
s3=s3.replace('static POWER_FLY_FORCE','pub static POWER_FLY_FORCE')
wr('spawner.rs',s3)
vx=rd('vfx.rs')
vx=ffi.sub(lambda m:'pub static KEY_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);\nunsafe fn GetAsyncKeyState(_vk: i32) -> i16 { if KEY_DOWN.load(Ordering::SeqCst) { 0x8000u16 as i16 } else { 0 } }\n',vx,count=1)
wr('vfx.rs',vx)
shutil.copy(S+'vfx_items.csv',D+'vfx_items.csv')
print('synced')
