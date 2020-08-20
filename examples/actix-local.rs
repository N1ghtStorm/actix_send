/*
    When enabling actix-runtime-local the actor is strictly run on single thread.

    It means the actor's address can not be send to other threads and no message from other threads
    can be handled.

    By default we don't enable actix runtime. Please run this example with:

    cargo run --example actix --no-default-features --features actix-runtime-local
*/

fn main() {
    #[cfg(feature = "actix-runtime-local")]
    fn _main() {
        use std::time::Instant;

        use actix_send::prelude::*;
        use futures_util::stream::{FuturesUnordered, StreamExt};
        use tokio::fs::File;
        use tokio::io::AsyncReadExt;

        #[actor(no_send)]
        pub struct ActixSendActor {
            pub file: File,
        }

        pub struct Ping;

        #[handler_v2(no_send)]
        impl ActixSendActor {
            async fn handle(&mut self, _: Ping) -> u8 {
                let mut buffer = [0u8; 1_000];
                let _ = self.file.read(&mut buffer).await.unwrap();
                1
            }
        }

        actix_rt::System::new("actix-test").block_on(async {
            let builder = ActixSendActor::builder(move || async move {
                let file = File::open("./sample/sample.txt").await.unwrap();
                ActixSendActor { file }
            });

            let address = builder.start().await;

            println!("starting benchmark actix_send");

            let join = (0..10000).fold(FuturesUnordered::new(), |f, _| {
                f.push(address.send(Ping));
                f
            });

            let start = Instant::now();
            let _ = join.collect::<Vec<_>>().await;
            println!(
                "total runtime is {:#?}",
                Instant::now().duration_since(start)
            );
        })
    }

    #[cfg(feature = "actix-runtime-local")]
    _main();
}
