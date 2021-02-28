pub(crate) use inner::{AsyncLock, AsyncLockGuard, Lock, RefCounter, WeakRefCounter};

mod inner {
    pub(crate) use std::sync::{Arc as RefCounter, Mutex, MutexGuard, Weak as WeakRefCounter};
    pub(crate) use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

    pub(crate) struct AsyncLock<T> {
        lock: AsyncMutex<T>,
    }

    pub(crate) type AsyncLockGuard<'a, T> = AsyncMutexGuard<'a, T>;

    impl<T: Default> Default for AsyncLock<T> {
        fn default() -> Self {
            AsyncLock {
                lock: AsyncMutex::default(),
            }
        }
    }

    impl<T> AsyncLock<T> {
        pub(crate) async fn lock(&self) -> AsyncLockGuard<'_, T> {
            self.lock.lock().await
        }
    }

    pub(crate) struct Lock<T> {
        lock: Mutex<T>,
    }

    pub(crate) type LockGuard<'a, T> = MutexGuard<'a, T>;

    impl<T> Lock<T> {
        pub(crate) fn new(value: T) -> Self {
            Self {
                lock: Mutex::new(value),
            }
        }

        pub(crate) fn lock(&self) -> LockGuard<'_, T> {
            self.lock.lock().unwrap()
        }
    }
}
