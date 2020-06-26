use std::future::Future;
use std::time::Duration;

macro_rules! spawn {
    (
        $spawn_fn: path,
        $delay_fn: path
    ) => {
        pub(crate) fn spawn<Fut>(f: Fut)
        where
            Fut: Future + Send + 'static,
            Fut::Output: Send + 'static,
        {
            $spawn_fn(f);
        }

        pub(crate) async fn delay_for(dur: Duration) {
            let _ = $delay_fn(dur).await;
        }
    };
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
spawn!(tokio::spawn, tokio::time::delay_for);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
spawn!(async_std::task::spawn, async_std::task::sleep);
