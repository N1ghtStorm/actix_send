use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use crate::util::runtime;

// helper function for spawn a future on runtime and return a handler that can cancel it.
pub(crate) fn spawn_cancelable<F>(f: F) -> FutureHandler
where
    F: Future + Unpin + Send + 'static,
{
    let waker = Arc::new(Mutex::new(None));

    let state = Arc::new(AtomicBool::new(true));

    let finisher = FinisherFuture {
        init: false,
        should_run: state.clone(),
        waker: waker.clone(),
    };

    runtime::spawn(async move {
        let _ = futures::future::select(finisher, f).await;
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
