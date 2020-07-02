use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::sync::{Mutex as AsyncMutex, MutexGuard};

use crate::actors::Actor;

// A shared hashmap of interval futures set between a set of actors.
#[derive(Clone)]
pub(crate) struct IntervalFutureSet<A>
where
    A: Actor,
{
    next_key: Arc<AtomicUsize>,
    futures: Arc<AsyncMutex<HashMap<usize, IntervalFuture<A>>>>,
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

    pub(crate) async fn lock(&self) -> MutexGuard<'_, HashMap<usize, IntervalFuture<A>>> {
        self.futures.lock().await
    }

    pub(crate) async fn insert(&self, future: IntervalFuture<A>) -> usize {
        let key = self.next_key.fetch_add(1, Ordering::SeqCst);
        self.futures.lock().await.insert(key, future);
        key
    }

    pub(crate) async fn remove(&self, key: usize) {
        self.futures.lock().await.remove(&key);
    }
}

/*
    The reason using a trait object for interval async closure is that
    we can register multiple intervals to one actor(or a set of shareable/cloneable actors)
*/

// a container for interval trait objects and a state for indicating if it's time to run this interval
pub(crate) struct IntervalFuture<A>
where
    A: Actor,
{
    func: Box<dyn IntervalFutureObj<A> + Send>,
}

impl<A> IntervalFuture<A>
where
    A: Actor,
{
    pub(crate) fn handle<'a>(
        &'a mut self,
        act: &'a mut A,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        self.func.as_mut().handle(act)
    }
}

// the trait object definition
trait IntervalFutureObj<A>
where
    A: Actor,
{
    // later in the pipeline we would want to extract the underlying async closure and pass actor state to it.
    fn handle<'a>(&'a mut self, act: &'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

// A type for containing the async closure.
pub(crate) struct IntervalFutureContainer<A, F>(pub(crate) F, pub(crate) PhantomData<A>)
where
    A: Actor + 'static,
    for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'static;

impl<A, F> IntervalFutureContainer<A, F>
where
    A: Actor + 'static,
    for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'static,
{
    pub(crate) fn pack(self) -> IntervalFuture<A> {
        IntervalFuture {
            func: Box::new(self),
        }
    }
}

// We make container type into a trait object.
impl<A, F> IntervalFutureObj<A> for IntervalFutureContainer<A, F>
where
    A: Actor + 'static,
    for<'a> F: Fn(&'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send + 'static,
{
    fn handle<'a>(&'a mut self, act: &'a mut A) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        (&mut self.0)(act)
    }
}
