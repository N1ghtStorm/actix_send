use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::fs::File;
use tokio::sync::Mutex;

use actix_send::prelude::*;

use crate::actix_actor::*;
use crate::actix_send_actor::*;

/*

    A naive benchmark between actix and actix_send.
    This example serve as a way to optimize actix_send crate.
    It DOES NOT represent real world performance for either crate.

    Run with:
    cargo build --example benchmark --release
    ./target/release/examples/benchmark --target <actix_send or actix> --rounds <usize>.

    optional argument: --heap-alloc

*/

fn main() {
    let num = num_cpus::get();

    let mut target = String::from("actix_send");
    let mut rounds = 1000;
    let mut heap_alloc = false;

    let mut iter = std::env::args().into_iter();

    let file_path = std::env::current_dir()
        .ok()
        .and_then(|path| {
            let path = path.to_str()?.to_owned();
            Some(path + "/sample/sample.txt")
        })
        .unwrap_or_else(|| String::from("./sample/sample.txt"));

    loop {
        if let Some(arg) = iter.next() {
            if arg.as_str() == "--target" {
                if let Some(arg) = iter.next() {
                    target = arg;
                }
            }
            if arg.as_str() == "--rounds" {
                if let Some(arg) = iter.next() {
                    if let Ok(r) = arg.parse::<usize>() {
                        rounds = r;
                    }
                }
            }
            if arg.as_str() == "--heap-alloc" {
                heap_alloc = true;
            }
            continue;
        }
        break;
    }

    match target.as_str() {
        "actix_send" => {
            println!("starting benchmark actix_send");

            tokio::runtime::Builder::new()
                .threaded_scheduler()
                .core_threads(num)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let file = simple_txt(file_path).await;

                    let actor = ActixSendActor::create(|| ActixSendActor { file, heap_alloc });

                    let address = actor.build().num(num).start();

                    let mut join = Vec::new();

                    for _ in 0..num {
                        for _ in 0..rounds {
                            join.push(address.send(PingSend));
                        }
                    }

                    let start = Instant::now();
                    futures::future::join_all(join).await;
                    println!(
                        "total runtime is {:#?}",
                        Instant::now().duration_since(start)
                    );
                    // delay the cold shutdown.
                    let _ = tokio::time::delay_for(Duration::from_secs(1)).await;
                });
        }
        "actix" => {
            println!("starting benchmark actix");

            actix_rt::System::new("actix").block_on(async move {
                let file = simple_txt(file_path).await;

                let mut join = Vec::new();

                for _ in 0..num {
                    let file = file.clone();
                    let heap_alloc = heap_alloc;
                    let arb = actix::Arbiter::new();
                    use actix::Actor;
                    let addr = ActixActor::start_in_arbiter(&arb, move |_| ActixActor {
                        file,
                        heap_alloc,
                    });

                    for _ in 0..rounds {
                        join.push(addr.send(Ping));
                    }
                }

                let start = Instant::now();
                let _ = futures::future::join_all(join).await;
                println!(
                    "total runtime is {:#?}",
                    Instant::now().duration_since(start)
                );
                let _ = tokio::time::delay_for(Duration::from_secs(1)).await;
            });
        }
        _ => panic!("--target must be either actix or actix_send"),
    }
}

async fn simple_txt(file_path: String) -> Arc<Mutex<File>> {
    let file = File::open(file_path).await.unwrap();
    Arc::new(Mutex::new(file))
}

#[actor_mod]
pub mod actix_send_actor {
    use std::sync::Arc;

    use tokio::fs::File;
    use tokio::io::AsyncReadExt;
    use tokio::sync::Mutex;

    use actix_send::prelude::*;

    #[actor]
    pub struct ActixSendActor {
        pub file: Arc<Mutex<File>>,
        pub heap_alloc: bool,
    }

    #[message(result = "u8")]
    pub struct PingSend;

    #[handler]
    impl Handler for ActixSendActor {
        async fn handle(&mut self, _: PingSend) -> u8 {
            // We can access &mut Self directly in async context like below.
            // But to be fair in comparison we clone the state and spawn a task.

            // if self.heap_alloc {
            //     let mut buffer = Vec::with_capacity(100_0000);
            //     let _ = self.file.lock().await.read(&mut buffer).await.unwrap();
            // } else {
            //     let mut buffer = [0u8; 1_000];
            //     let _ = self.file.lock().await.read(&mut buffer).await.unwrap();
            // }

            let f = self.file.clone();
            let heap = self.heap_alloc;
            tokio::spawn(async move {
                if heap {
                    let mut buffer = Vec::with_capacity(100_0000);
                    let _ = f.lock().await.read(&mut buffer).await.unwrap();
                } else {
                    let mut buffer = [0u8; 1_000];
                    let _ = f.lock().await.read(&mut buffer).await.unwrap();
                }
            });
            1
        }
    }
}

pub mod actix_actor {
    use std::sync::Arc;

    use actix::{Actor, AsyncContext, Context, Handler, Message, WrapFuture};
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;
    use tokio::sync::Mutex;

    pub struct ActixActor {
        pub file: Arc<Mutex<File>>,
        pub heap_alloc: bool,
    }

    impl Actor for ActixActor {
        type Context = Context<Self>;
    }

    pub struct Ping;

    impl Message for Ping {
        type Result = u8;
    }

    impl Handler<Ping> for ActixActor {
        type Result = u8;

        fn handle(&mut self, _: Ping, ctx: &mut Context<Self>) -> Self::Result {
            let f = self.file.clone();
            let heap = self.heap_alloc;
            ctx.spawn(
                async move {
                    if heap {
                        let mut buffer = Vec::with_capacity(100_0000);
                        let _ = f.lock().await.read(&mut buffer).await.unwrap();
                    } else {
                        let mut buffer = [0u8; 1_000];
                        let _ = f.lock().await.read(&mut buffer).await.unwrap();
                    }
                }
                .into_actor(self),
            );
            1
        }
    }
}
