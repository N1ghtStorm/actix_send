#[cfg(feature = "actix-runtime")]
use {
    crate::actix_actor::*,
    crate::actix_send_actor::*,
    actix::Arbiter,
    actix_send::prelude::*,
    futures_util::{stream::FuturesUnordered, FutureExt, StreamExt},
    std::cell::RefCell,
    std::rc::Rc,
    std::time::Instant,
    tokio::fs::File,
    tokio::io::AsyncReadExt,
};

/*

    A naive benchmark between actix and actix_send.
    This example serve as a way to optimize actix_send crate.

    It DOES NOT represent the real world performance of either crate.

    Build with:

    cargo build --example benchmark --no-default-features --features actix-runtime --release


    Run with:

    ./target/release/examples/benchmark --target <actix_send or actix> --rounds <usize>.

    optional argument: --heap-alloc
                       --dynamic

*/

fn main() {
    #[cfg(feature = "actix-runtime")]
    actix_rt::System::new("benchmark").block_on(async {
        let num = num_cpus::get();

        let mut target = String::from("actix_send");
        let mut rounds = 1000;
        let mut heap_alloc = false;
        let mut dynamic = false;

        let file_path = collect_arg(&mut target, &mut rounds, &mut heap_alloc, &mut dynamic);

        match target.as_str() {
            "actix_send" => {
                let builder = ActixSendActor::builder(move || {
                    let file_path = file_path.clone();
                    async move {
                        let file = File::open(file_path).await.unwrap();
                        ActixSendActor { file, heap_alloc }
                    }
                });

                let arbiters = (0..num).map(|_| Arbiter::new()).collect::<Vec<Arbiter>>();

                let address = builder.num(num).start_with_arbiters(&arbiters, None).await;

                if dynamic {
                    println!("starting benchmark actix_send with dynamic dispatch");

                    let join = (0..num * rounds).fold(FuturesUnordered::new(), |f, _| {
                        f.push(address.run(|actor| actor.read_file().boxed()));
                        f
                    });

                    let start = Instant::now();
                    let _ = join.collect::<Vec<_>>().await;
                    println!(
                        "total runtime is {:#?}",
                        Instant::now().duration_since(start)
                    );
                } else {
                    println!("starting benchmark actix_send");

                    let join = (0..num * rounds).fold(FuturesUnordered::new(), |f, _| {
                        f.push(address.send(Ping));
                        f
                    });

                    let start = Instant::now();
                    let _ = join.collect::<Vec<_>>().await;
                    println!(
                        "total runtime is {:#?}",
                        Instant::now().duration_since(start)
                    );
                };
            }
            "actix" => {
                println!("starting benchmark actix");

                let join = FuturesUnordered::new();

                for _ in 0..num {
                    let file = File::open(file_path.clone()).await.unwrap();
                    let heap_alloc = heap_alloc;
                    let arb = actix::Arbiter::new();
                    use actix::Actor;
                    let addr = ActixActor::start_in_arbiter(&arb, move |_| ActixActor {
                        file: Rc::new(RefCell::new(file)),
                        heap_alloc,
                    });

                    for _ in 0..rounds {
                        join.push(addr.send(Ping));
                    }
                }

                let start = Instant::now();
                let _ = join.collect::<Vec<_>>().await;
                println!(
                    "total runtime is {:#?}",
                    Instant::now().duration_since(start)
                );
            }
            _ => panic!("--target must be either actix or actix_send"),
        }
    });
}

#[cfg(feature = "actix-runtime")]
pub struct Ping;

#[cfg(feature = "actix-runtime")]
pub mod actix_send_actor {
    use super::*;

    #[actor]
    pub struct ActixSendActor {
        pub file: File,
        pub heap_alloc: bool,
    }

    impl ActixSendActor {
        pub async fn read_file(&mut self) -> u8 {
            read_file(&mut self.file, self.heap_alloc).await
        }
    }

    #[handler_v2]
    impl ActixSendActor {
        async fn handle(&mut self, _: Ping) -> u8 {
            self.read_file().await
        }
    }
}

#[cfg(feature = "actix-runtime")]
pub mod actix_actor {
    use actix::{Actor, AtomicResponse, Context, Handler, Message, WrapFuture};

    use super::*;

    pub struct ActixActor {
        pub file: Rc<RefCell<File>>,
        pub heap_alloc: bool,
    }

    impl Actor for ActixActor {
        type Context = Context<Self>;
    }

    impl Message for Ping {
        type Result = u8;
    }

    impl Handler<Ping> for ActixActor {
        type Result = AtomicResponse<Self, u8>;

        fn handle(&mut self, _: Ping, _ctx: &mut Context<Self>) -> Self::Result {
            let f = self.file.clone();
            let heap = self.heap_alloc;

            let fut = Box::pin(
                async move {
                    let mut refmut = f.borrow_mut();

                    read_file(&mut refmut, heap).await
                }
                .into_actor(self),
            );

            AtomicResponse::new(fut)
        }
    }
}

#[cfg(feature = "actix-runtime")]
async fn read_file(file: &mut File, heap: bool) -> u8 {
    if heap {
        let mut buffer = Vec::with_capacity(100_0000);
        let _ = file.read(&mut buffer).await.unwrap();
    } else {
        let mut buffer = [0u8; 1_000];
        let _ = file.read(&mut buffer).await.unwrap();
    }
    1
}

#[cfg(feature = "actix-runtime")]
fn collect_arg(
    target: &mut String,
    rounds: &mut usize,
    heap_alloc: &mut bool,
    dynamic: &mut bool,
) -> String {
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
                    *target = arg;
                }
            }
            if arg.as_str() == "--rounds" {
                if let Some(arg) = iter.next() {
                    if let Ok(r) = arg.parse::<usize>() {
                        *rounds = r;
                    }
                }
            }
            if arg.as_str() == "--heap-alloc" {
                *heap_alloc = true;
            }
            if arg.as_str() == "--dynamic" {
                *dynamic = true;
            }
            continue;
        }
        break;
    }

    file_path
}
