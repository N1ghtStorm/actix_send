use std::future::Future;

macro_rules! spawn {
    ($spawn_fn: path) => {
        pub(crate) fn spawn<Fut>(f: Fut)
        where
            Fut: Future + Send + 'static,
            Fut::Output: Send + 'static,
        {
            $spawn_fn(f);
        }
    };
}

#[cfg(feature = "tokio-runtime")]
#[cfg(not(feature = "async-std-runtime"))]
spawn!(tokio::spawn);

#[cfg(feature = "async-std-runtime")]
#[cfg(not(feature = "tokio-runtime"))]
spawn!(async_std::task::spawn);
