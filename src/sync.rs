//! A mutex that survives a panic in another thread.
//!
//! `Mutex::lock().unwrap()` turns one panic while a lock is held into a panic
//! at every later use of that lock: a poisoned status, history or settings
//! mutex would take the whole web console down while the service itself keeps
//! running. The state behind these locks is plain data that stays usable, so
//! the receiver carries on with what the panicking thread left.
use std::sync::{Mutex, MutexGuard, PoisonError};

pub trait LockExt<T> {
    /// `lock()`, ignoring poisoning.
    fn lock_ok(&self) -> MutexGuard<'_, T>;
}
impl<T> LockExt<T> for Mutex<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_panic_while_locked_does_not_poison_later_users() {
        let m = Arc::new(Mutex::new(5));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _guard = m2.lock().unwrap();
            panic!("a thread dies holding the lock");
        })
        .join();
        assert!(m.lock().is_err(), "plain lock() is poisoned");
        assert_eq!(*m.lock_ok(), 5, "lock_ok() still works");
        *m.lock_ok() += 1;
        assert_eq!(*m.lock_ok(), 6);
    }
}
