use core::sync::atomic::{AtomicUsize, Ordering};

use std::collections::HashMap;
use std::sync::Arc;

use crate::actor::Actor;
use crate::object::FutureObjectContainer;
use crate::util::smart_pointer::{AsyncLock, AsyncLockGuard};

// A shared hashmap of interval futures set between a set of actors.
pub(crate) struct IntervalFutureSet<A>
where
    A: Actor,
{
    next_key: Arc<AtomicUsize>,
    futures: Arc<AsyncLock<HashMap<usize, FutureObjectContainer<A>>>>,
}

impl<A> Clone for IntervalFutureSet<A>
where
    A: Actor,
{
    fn clone(&self) -> Self {
        Self {
            next_key: self.next_key.clone(),
            futures: self.futures.clone(),
        }
    }
}

impl<A> Default for IntervalFutureSet<A>
where
    A: Actor,
{
    fn default() -> Self {
        Self {
            next_key: Arc::new(AtomicUsize::new(0)),
            futures: Default::default(),
        }
    }
}

impl<A> IntervalFutureSet<A>
where
    A: Actor,
{
    pub(crate) async fn lock(
        &self,
    ) -> AsyncLockGuard<'_, HashMap<usize, FutureObjectContainer<A>>> {
        self.futures.lock().await
    }

    pub(crate) async fn insert(&self, future: FutureObjectContainer<A>) -> usize {
        let key = self.next_key.fetch_add(1, Ordering::Relaxed);

        assert!(key < std::usize::MAX, "Too many interval futures");

        self.futures.lock().await.insert(key, future);

        key
    }

    pub(crate) async fn remove(&self, key: usize) {
        self.futures.lock().await.remove(&key);
    }
}
