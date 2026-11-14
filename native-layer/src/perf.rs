//! Time spent in the places the game calls the mod from: how many calls, the total and the worst. Nothing
//! runs on a timer: a call that finds the interval over writes one line, and only when some call took
//! longer than `SLOW` in that interval.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const CREATE: usize = 0;
pub const QUESTION: usize = 1;
pub const TICK: usize = 2;
pub const SHIELD: usize = 3;
pub const PERMISSION: usize = 4;
const SLOTS: usize = 5;
const NAMES: [&str; SLOTS] = ["entity create", "relation question", "groups tick", "shield work", "permission lookup"];
const SLOW: Duration = Duration::from_millis(5);
const EVERY_MS: u64 = 30_000;
const SAMPLE_EVERY: u64 = 16;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static CALLS: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static TIMED: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static TOTAL_US: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static WORST_US: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static SLOW_CALLS: AtomicU64 = AtomicU64::new(0);
static REPORT_AT: AtomicU64 = AtomicU64::new(EVERY_MS);
static START: OnceLock<Instant> = OnceLock::new();

pub struct Span(usize, Instant);

/// Times everything from here to the end of the scope.
pub fn span(slot: usize) -> Span {
    CALLS[slot].fetch_add(1, Ordering::Relaxed);
    Span(slot, Instant::now())
}

/// For calls that come thousands of times a frame: every call is counted, one in sixteen is timed.
pub fn sampled(slot: usize) -> Option<Span> {
    let n = CALLS[slot].fetch_add(1, Ordering::Relaxed);
    (n % SAMPLE_EVERY == 0).then(|| Span(slot, Instant::now()))
}

impl Drop for Span {
    fn drop(&mut self) {
        record(self.0, self.1.elapsed());
    }
}

fn record(slot: usize, took: Duration) {
    let us = took.as_micros() as u64;
    TIMED[slot].fetch_add(1, Ordering::Relaxed);
    TOTAL_US[slot].fetch_add(us, Ordering::Relaxed);
    WORST_US[slot].fetch_max(us, Ordering::Relaxed);
    if took >= SLOW {
        SLOW_CALLS.fetch_add(1, Ordering::Relaxed);
    }
    let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let due = REPORT_AT.load(Ordering::Relaxed);
    if now >= due && REPORT_AT.compare_exchange(due, now + EVERY_MS, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        report();
    }
}

fn report() {
    let slow = SLOW_CALLS.swap(0, Ordering::Relaxed);
    let mut parts = Vec::new();
    for slot in 0..SLOTS {
        let calls = CALLS[slot].swap(0, Ordering::Relaxed);
        let timed = TIMED[slot].swap(0, Ordering::Relaxed);
        let total = TOTAL_US[slot].swap(0, Ordering::Relaxed);
        let worst = WORST_US[slot].swap(0, Ordering::Relaxed);
        if calls > 0 && timed > 0 {
            parts.push(format!("{} {} calls, {} ms timed, worst {} ms", NAMES[slot], calls, total / 1000, worst / 1000));
        }
    }
    if slow > 0 {
        super::spawner::log(&format!("perf: {slow} call(s) took 5 ms or more in the last 30 s: {}", parts.join("; ")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_every_sixteenth_call_is_timed() {
        let timed = (0..64).filter(|_| sampled(QUESTION).is_some()).count();
        assert_eq!(timed, 4);
    }
}
