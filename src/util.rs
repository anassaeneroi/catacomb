//! Small shared helpers.

use std::sync::{Mutex, MutexGuard};

/// Lock a [`Mutex`], recovering the inner data if the lock was poisoned by
/// a panic in another thread while it was held.
///
/// The web server holds several long-lived `Mutex`es in `WebState`. With
/// the default `.lock().unwrap()`, a single panic while one of them is held
/// would poison it and turn *every* subsequent access into a panic —
/// cascading one handler's bug into a permanently dead server. Recovering
/// via [`std::sync::PoisonError::into_inner`] keeps it serving: our
/// critical sections are short and don't leave half-updated invariants a
/// later reader would choke on, so the data behind the lock is still
/// usable, and one stuck request is far better than a stuck process.
pub trait LockExt<T> {
    /// Acquire the guard, recovering from poisoning instead of panicking.
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn recovers_after_a_panic_poisoned_the_lock() {
        let m = Arc::new(Mutex::new(41));
        // Poison the mutex: panic inside a thread while holding the guard.
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("boom while holding the lock");
        })
        .join();
        // Plain lock() would now return Err(PoisonError). lock_recover
        // hands back the data so the server keeps going.
        assert!(m.lock().is_err(), "precondition: mutex is poisoned");
        *m.lock_recover() += 1;
        assert_eq!(*m.lock_recover(), 42);
    }
}
