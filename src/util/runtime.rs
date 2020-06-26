use std::future::Future;
use std::time::Duration;

macro_rules! spawn {
    (
        $spawn_fn: path,
        $delay_fn: path,
        $interval_fn: path,
        $interval_ty: path
    ) => {
        pub(crate) type Interval = $interval_ty;

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

        pub(crate) fn interval(dur: Duration) -> Interval {
            $interval_fn(dur)
        }
    };
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
spawn!(
    tokio::spawn,
    tokio::time::delay_for,
    tokio::time::interval,
    tokio::time::Interval
);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
spawn!(async_std::task::spawn, async_std::task::sleep);
