/*
    The reason using a trait object for running async closure is that
    We can register multiple async closure with different return types to one actor.
*/

use core::any::Any;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;

use crate::actor::Actor;

macro_rules! object_container {
    ($($send:ident)*) => {
        // a container for FutureTrait object
        pub(crate) struct FutureObjectContainer<A>
        where
            A: Actor,
        {
            func: Box<dyn FutureTrait<A> $( + $send)*>,
        }
    }
}

#[cfg(not(feature = "actix-runtime-local"))]
object_container!(Send);

#[cfg(feature = "actix-runtime-local")]
object_container!();

macro_rules! object {
    ($($send:ident)*) => {
        impl<A> FutureObjectContainer<A>
        where
            A: Actor,
        {
            // we call this method in Actor's context loop.
            pub(crate) fn handle<'a>(
                &'a mut self,
                act: &'a mut A,
            ) -> Pin<Box<dyn Future<Output = AnyObjectContainer> $( + $send)* + 'a>> {
                self.func.handle(act)
            }
        }

        // the trait definition
        // We return another contained trait object in the output.
        // So that we can send it through Oneshot channel without additional type signature.
        trait FutureTrait<A>
        where
            A: Actor,
        {
            fn handle<'a>(
                &'a mut self,
                act: &'a mut A,
            ) -> Pin<Box<dyn Future<Output = AnyObjectContainer> $( + $send)* + 'a>>;
        }

        // The type we want to implement FutureTrait.
        pub(crate) struct FutureObject<A, F, R>(
            pub(crate) F,
            pub(crate) PhantomData<R>,
            pub(crate) PhantomData<A>,
        )
        where
            A: Actor + 'static,
            F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = R> $( + $send)* + '_>> + Send + 'static,
            R: Send + 'static;

        impl<A, F, R> FutureObject<A, F, R>
        where
            A: Actor + 'static,
            F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = R> $( + $send)* + '_>> + Send + 'static,
            R: Send + 'static,
        {
            pub(crate) fn pack(self) -> FutureObjectContainer<A> {
                FutureObjectContainer {
                    func: Box::new(self),
                }
            }
        }

        impl<A, F, R> FutureTrait<A> for FutureObject<A, F, R>
        where
            A: Actor + 'static,
            F: FnMut(&mut A) -> Pin<Box<dyn Future<Output = R> $( + $send)* + '_>> + Send + 'static,
            R: Send + 'static,
        {
            fn handle<'a>(
                &'a mut self,
                act: &'a mut A,
            ) -> Pin<Box<dyn Future<Output = AnyObjectContainer> $( + $send)* + 'a>> {
                let fut = (&mut self.0)(act);
                Box::pin(async move {
                    let r = fut.await;
                    AnyObjectContainer::pack(r)
                })
            }
        }
    };
}

#[cfg(not(any(feature = "actix-runtime", feature = "actix-runtime-local")))]
object!(Send);

#[cfg(any(feature = "actix-runtime", feature = "actix-runtime-local"))]
object!();

// A containner type for packing and unpacking a type to/from a Any trait object
pub(crate) struct AnyObjectContainer {
    inner: Box<dyn Any + Send>,
}

impl AnyObjectContainer {
    pub(crate) fn pack<R>(r: R) -> Self
    where
        R: Send + 'static,
    {
        Self {
            inner: Box::new(Some(r)),
        }
    }

    // We convert future result trait object into type using downcast.
    pub(crate) fn unpack<R>(&mut self) -> Option<R>
    where
        R: 'static,
    {
        self.inner.downcast_mut::<Option<R>>()?.take()
    }
}
