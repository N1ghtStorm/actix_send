use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::sync::{Mutex as AsyncMutex, MutexGuard};

use crate::actors::Actor;
use crate::object::FutureObjectContainer;

// A shared hashmap of interval futures set between a set of actors.
#[derive(Clone)]
pub(crate) struct IntervalFutureSet<A>
where
    A: Actor,
{
    next_key: Arc<AtomicUsize>,
    futures: Arc<AsyncMutex<HashMap<usize, FutureObjectContainer<A>>>>,
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
    pub(crate) fn new() -> Self {
        Default::default()
    }

    pub(crate) async fn lock(&self) -> MutexGuard<'_, HashMap<usize, FutureObjectContainer<A>>> {
        self.futures.lock().await
    }

    pub(crate) async fn insert(&self, future: FutureObjectContainer<A>) -> usize {
        let key = self.next_key.fetch_add(1, Ordering::SeqCst);
        self.futures.lock().await.insert(key, future);
        key
    }

    pub(crate) async fn remove(&self, key: usize) {
        self.futures.lock().await.remove(&key);
    }
}
