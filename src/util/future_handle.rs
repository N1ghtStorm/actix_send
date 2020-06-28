use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use crate::util::runtime;
use futures::future::Either;

// helper function for spawn a future on runtime and return a handler that can cancel it.
pub(crate) fn spawn_cancelable<F, FN>(f: F, cancel_now: bool, on_cancel: FN) -> FutureHandler
where
    F: Future + Unpin + Send + 'static,
    <F as Future>::Output: Send,
    FN: Fn() + Send + 'static,
{
    let waker = Arc::new(Mutex::new(None));

    let state = Arc::new(AtomicBool::new(true));

    let finisher = FinisherFuture {
        init: false,
        should_run: state.clone(),
        waker: waker.clone(),
    };

    runtime::spawn(async move {
        let either = futures::future::select(finisher, f).await;

        if let Either::Left((_, f)) = either {
            on_cancel();
            if !cancel_now {
                let _ = f.await;
            }
        }
    });

    FutureHandler { state, waker }
}

// a future notified and polled by future_handler.
pub(crate) struct FinisherFuture {
    init: bool,
    should_run: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl Future for FinisherFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if !this.should_run.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        if !this.init {
            let mut waker = this.waker.lock();
            *waker = Some(cx.waker().clone());
            this.init = true;
        }

        Poll::Pending
    }
}

pub struct FutureHandler {
    state: Arc<AtomicBool>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl FutureHandler {
    /// Cancel the future.
    pub fn cancel(&self) {
        self.state.store(false, Ordering::SeqCst);
        if let Some(waker) = self.waker.lock().take() {
            waker.wake();
        }
    }
}
