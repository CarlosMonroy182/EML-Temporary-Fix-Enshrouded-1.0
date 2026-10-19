//! Keeps the game's mouse and keyboard working while the panel's window exists.

use std::sync::Mutex;

pub const REMOVE: u32 = 0x0000_0001;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Raw {
    pub page: u16,
    pub usage: u16,
    pub flags: u32,
    pub target: usize,
}

pub trait RawSys: Send + Sync + 'static {
    fn registered(&self) -> Vec<Raw>;
    fn is_foreign(&self, target: usize) -> bool;
    fn register(&self, devices: &[Raw]) -> bool;
}

pub struct RawGuard<S: RawSys> {
    sys: S,
    saved: Mutex<Vec<Raw>>,
}

impl<S: RawSys> RawGuard<S> {
    pub fn new(sys: S) -> Self {
        RawGuard { sys, saved: Mutex::new(Vec::new()) }
    }

    /// Returns true when it had to give the devices back to the game.
    pub fn tick(&self) -> bool {
        let current = self.sys.registered();
        if current.is_empty() {
            return false;
        }
        let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
        let mut taken = Vec::new();
        for entry in &current {
            if entry.target != 0 && self.sys.is_foreign(entry.target) {
                taken.push(*entry);
            } else if let Some(slot) = saved.iter_mut().find(|s| s.page == entry.page && s.usage == entry.usage) {
                *slot = *entry;
            } else {
                saved.push(*entry);
            }
        }
        if taken.is_empty() {
            return false;
        }
        let back: Vec<Raw> = taken
            .iter()
            .map(|t| {
                saved
                    .iter()
                    .copied()
                    .find(|s| s.page == t.page && s.usage == t.usage)
                    .unwrap_or(Raw { page: t.page, usage: t.usage, flags: REMOVE, target: 0 })
            })
            .collect();
        self.sys.register(&back)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Fake {
        state: Mutex<Vec<Raw>>,
    }

    impl RawSys for Arc<Fake> {
        fn registered(&self) -> Vec<Raw> {
            self.state.lock().unwrap().clone()
        }
        fn is_foreign(&self, target: usize) -> bool {
            target >= 500
        }
        fn register(&self, devices: &[Raw]) -> bool {
            let mut state = self.state.lock().unwrap();
            for d in devices {
                state.retain(|s| !(s.page == d.page && s.usage == d.usage));
                if d.flags & REMOVE == 0 {
                    state.push(*d);
                }
            }
            true
        }
    }

    fn game() -> Vec<Raw> {
        vec![
            Raw { page: 1, usage: 2, flags: 0, target: 100 },
            Raw { page: 1, usage: 6, flags: 0x30, target: 100 },
        ]
    }

    #[test]
    fn gives_the_devices_back_when_the_panel_library_takes_them() {
        let fake = Arc::new(Fake { state: Mutex::new(game()) });
        let guard = RawGuard::new(fake.clone());
        assert!(!guard.tick());
        fake.register(&[
            Raw { page: 1, usage: 2, flags: 0x2000, target: 900 },
            Raw { page: 1, usage: 6, flags: 0x2000, target: 900 },
        ]);
        assert!(guard.tick());
        let mut now = fake.registered();
        now.sort_by_key(|r| r.usage);
        assert_eq!(now, game());
        assert!(!guard.tick());
    }

    #[test]
    fn follows_the_game_when_it_registers_something_else_itself() {
        let fake = Arc::new(Fake { state: Mutex::new(game()) });
        let guard = RawGuard::new(fake.clone());
        guard.tick();
        fake.register(&[Raw { page: 1, usage: 2, flags: 0, target: 200 }]);
        assert!(!guard.tick());
        fake.register(&[Raw { page: 1, usage: 2, flags: 0x2000, target: 900 }]);
        assert!(guard.tick());
        assert!(fake.registered().contains(&Raw { page: 1, usage: 2, flags: 0, target: 200 }));
    }

    #[test]
    fn removes_a_registration_the_game_never_had() {
        let fake = Arc::new(Fake { state: Mutex::new(vec![Raw { page: 1, usage: 6, flags: 0, target: 100 }]) });
        let guard = RawGuard::new(fake.clone());
        guard.tick();
        fake.register(&[Raw { page: 1, usage: 2, flags: 0, target: 900 }]);
        assert!(guard.tick());
        assert_eq!(fake.registered(), vec![Raw { page: 1, usage: 6, flags: 0, target: 100 }]);
    }
}
