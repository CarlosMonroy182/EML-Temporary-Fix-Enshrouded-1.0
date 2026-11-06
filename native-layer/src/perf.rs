use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const POWER: usize = 0;
pub const POLL: usize = 1;
pub const GROUPS: usize = 2;
pub const SPAWNERS: usize = 3;
pub const DELIVERIES: usize = 4;
pub const REFILL: usize = 5;
pub const SHIELD: usize = 6;
pub const CHAT: usize = 7;
pub const ORDERS: usize = 8;
pub const ORDER_WAIT: usize = 9;
const SLOTS: usize = 10;
const NAMES: [&str; SLOTS] = ["power loop", "file polls", "teams and zones", "spawners", "deliveries", "refills", "shield", "chat lines", "orders run", "orders waited"];

const ZERO: AtomicU64 = AtomicU64::new(0);
static NANOS: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static CALLS: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static WORST: [AtomicU64; SLOTS] = [ZERO; SLOTS];

pub fn time<R>(slot: usize, work: impl FnOnce() -> R) -> R {
    let start = Instant::now();
    let result = work();
    let spent = start.elapsed().as_nanos() as u64;
    NANOS[slot].fetch_add(spent, Ordering::Relaxed);
    CALLS[slot].fetch_add(1, Ordering::Relaxed);
    WORST[slot].fetch_max(spent, Ordering::Relaxed);
    result
}

/// Adds a duration measured elsewhere (for example how long an order waited in the queue).
pub fn add(slot: usize, nanos: u64) {
    NANOS[slot].fetch_add(nanos, Ordering::Relaxed);
    CALLS[slot].fetch_add(1, Ordering::Relaxed);
    WORST[slot].fetch_max(nanos, Ordering::Relaxed);
}

pub fn report() -> Option<String> {
    let mut parts = Vec::new();
    for (slot, name) in NAMES.iter().enumerate() {
        let calls = CALLS[slot].swap(0, Ordering::Relaxed);
        let nanos = NANOS[slot].swap(0, Ordering::Relaxed);
        let worst = WORST[slot].swap(0, Ordering::Relaxed);
        if calls > 0 {
            parts.push(format!(
                "{name}: {calls} calls, {:.1} ms total, worst {:.2} ms",
                nanos as f64 / 1e6,
                worst as f64 / 1e6
            ));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("performance in the last minute: {}", parts.join("; ")))
    }
}
