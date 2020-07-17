// ToDo: it could be a better idea to use a mature lock crate.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

// bits for hint the lock state.
const FREE: usize = 0;
const CLOSED: usize = 1;
const LOCKED: usize = 1 << 1;

pub struct SimpleSpinLock<T> {
    state: AtomicUsize,
    value: UnsafeCell<T>,
}

// value of lock is only accessed through guard.
pub struct SimpleSpinGuard<'a, T> {
    state: &'a AtomicUsize,
    value: &'a mut T,
}

impl<T> Deref for SimpleSpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T> DerefMut for SimpleSpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

// when dropping guard we write state to free regardless if the lock is closed.
impl<T> Drop for SimpleSpinGuard<'_, T> {
    fn drop(&mut self) {
        self.state.store(FREE, Ordering::Release);
        // # Safety: Any operation of closing the lock must come after drop of SimpleSpinGuard.
    }
}

impl<T> SimpleSpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicUsize::new(FREE),
            value: UnsafeCell::new(value),
        }
    }

    // We return None if the lock is already closed.
    pub fn lock(&self) -> Option<SimpleSpinGuard<'_, T>> {
        let mut state = self.state.load(Ordering::Relaxed);

        loop {
            if state & CLOSED == 1 {
                return None;
            }

            match self.state.compare_exchange_weak(
                FREE,
                LOCKED,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // # Safety:
                    // We only return a guard when we exchange the state from FREE to LOCKED
                    return Some(SimpleSpinGuard {
                        state: &self.state,
                        value: unsafe { &mut *self.value.get() },
                    });
                }
                Err(s) => state = s,
            }
        }
    }

    // # Safety: Any operation of closing the lock must come after drop of SimpleSpinGuard.
    // In other word we only close the lock when the state is FREE
    pub fn close(&self) -> bool {
        let mut state = self.state.load(Ordering::Relaxed);

        loop {
            if state & CLOSED != 0 {
                return false;
            }

            match self.state.compare_exchange_weak(
                FREE,
                CLOSED,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(s) => state = s,
            }
        }
    }
}

unsafe impl<T> Send for SimpleSpinLock<T> where T: Send {}

unsafe impl<T> Sync for SimpleSpinLock<T> where T: Send {}
