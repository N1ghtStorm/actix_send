use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures::Stream;
use parking_lot::Mutex;

use crate::actors::Actor;
use crate::util::constant::{ACTIVE, SHUT, SLEEP};
use crate::util::runtime;

// helper function to construct stream and handler for IntervalFuture
pub(crate) fn interval_future_handler(
    state: Arc<AtomicUsize>,
    dur: Duration,
) -> (IntervalFutureStream, IntervalFutureHandler) {
    let waker = Arc::new(Mutex::new(None));

    let checker = IntervalFutureStream {
        init: false,
        state: state.clone(),
        waker: waker.clone(),
        interval: runtime::interval(dur),
    };

    let handler = IntervalFutureHandler { state, waker };

    (checker, handler)
}

// this is just a checker for the state of interval.
pub(crate) struct IntervalFutureStream {
    init: bool,
    state: Arc<AtomicUsize>,
    waker: Arc<Mutex<Option<Waker>>>,
    interval: runtime::Interval,
}

impl Stream for IntervalFutureStream {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let state = this.state.load(Ordering::Acquire);

        if state == SHUT {
            return Poll::Ready(None);
        }

        if !this.init {
            let mut waker = this.waker.lock();
            *waker = Some(cx.waker().clone());
            this.init = true;
        }

        match this.interval.poll_tick(cx) {
            Poll::Ready(_) => {
                this.state.store(ACTIVE, Ordering::Release);
                Poll::Ready(Some(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct IntervalFutureHandler {
    state: Arc<AtomicUsize>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl IntervalFutureHandler {
    /// Cancel the interval future.
    pub fn cancel(&self) {
        self.state.store(SHUT, Ordering::SeqCst);
        if let Some(waker) = self.waker.lock().take() {
            waker.wake();
        }
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
    pub(crate) state: Arc<AtomicUsize>,
    func: Box<dyn IntervalFutureObj<A> + Send>,
}

impl<A> IntervalFuture<A>
where
    A: Actor,
{
    pub(crate) fn handle(&mut self, actor: A) -> Pin<Box<dyn Future<Output = A> + Send>> {
        self.func.as_mut().handle(actor)
    }
}

// the trait object definition
trait IntervalFutureObj<A>
where
    A: Actor,
{
    // later in the pipeline we would want to extract the underlying async closure and pass actor state to it.
    fn handle(&mut self, act: A) -> Pin<Box<dyn Future<Output = A> + Send>>;
}

// A type for containing the async closure.
pub(crate) struct IntervalFutureContainer<A, F, Fut>(
    pub(crate) F,
    pub(crate) PhantomData<A>,
    pub(crate) PhantomData<Fut>,
)
where
    A: Actor + 'static,
    F: Fn(A) -> Fut + Send + 'static,
    Fut: Future<Output = A> + Send + 'static;

impl<A, F, Fut> IntervalFutureContainer<A, F, Fut>
where
    A: Actor + 'static,
    F: Fn(A) -> Fut + Send + 'static,
    Fut: Future<Output = A> + Send + 'static,
{
    pub(crate) fn pack(self) -> IntervalFuture<A> {
        IntervalFuture {
            state: Arc::new(AtomicUsize::new(SLEEP)),
            func: Box::new(self),
        }
    }
}

// We make container type into a trait object.
impl<A, F, Fut> IntervalFutureObj<A> for IntervalFutureContainer<A, F, Fut>
where
    A: Actor + 'static,
    F: Fn(A) -> Fut + Send + 'static,
    Fut: Future<Output = A> + Send + 'static,
{
    fn handle(&mut self, act: A) -> Pin<Box<dyn Future<Output = A> + Send>> {
        Box::pin((&mut self.0)(act))
    }
}
