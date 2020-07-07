/*
    The reason using a trait object for running async closure is that
    We can register multiple async closure with different return types to one actor.
*/

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use crate::actor::Actor;

// a container for FutureTrait object
pub(crate) struct FutureObjectContainer<A>
where
    A: Actor,
{
    func: Box<dyn FutureTrait<A> + Send>,
}

impl<A> FutureObjectContainer<A>
where
    A: Actor,
{
    // we call this method in Actor's context loop.
    pub(crate) fn handle<'a>(
        &'a mut self,
        act: &'a mut A,
    ) -> Pin<Box<dyn Future<Output = FutureResultObjectContainer> + Send + 'a>> {
        self.func.as_mut().handle(act)
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
    ) -> Pin<Box<dyn Future<Output = FutureResultObjectContainer> + Send + 'a>>;
}

// The type we want to implement FutureTrait.
pub(crate) struct FutureObject<A, F, R>(
    pub(crate) F,
    pub(crate) PhantomData<R>,
    pub(crate) PhantomData<A>,
)
where
    A: Actor + 'static,
    for<'a> F: FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = R> + Send + 'a>> + Send + 'static,
    R: Send + 'static;

impl<A, F, R> FutureObject<A, F, R>
where
    A: Actor + 'static,
    for<'a> F: FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = R> + Send + 'a>> + Send + 'static,
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
    for<'a> F: FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = R> + Send + 'a>> + Send + 'static,
    R: Send + 'static,
{
    fn handle<'a>(
        &'a mut self,
        act: &'a mut A,
    ) -> Pin<Box<dyn Future<Output = FutureResultObjectContainer> + Send + 'a>> {
        let fut = (&mut self.0)(act);
        Box::pin(async move {
            let r = fut.await;
            FutureResultObjectContainer::pack(r)
        })
    }
}

// A type contain the result trait object.
pub(crate) struct FutureResultObjectContainer {
    result: Box<dyn FutureResultTrait + Send>,
}

impl FutureResultObjectContainer {
    fn pack<R>(r: R) -> Self
    where
        R: Send + 'static,
    {
        Self {
            result: Box::new(Some(r)),
        }
    }

    // We convert future result trait object into type using downcast.
    pub(crate) fn unpack<R>(&mut self) -> Option<R>
    where
        R: 'static,
    {
        self.result.as_any_mut().downcast_mut::<Option<R>>()?.take()
    }
}

// We have to map our trait object into Any in order to downcast it.
trait FutureResultTrait {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

// We impl FutureResultTrait for Option<R> instead of R because we want to take the inner data out.
// Otherwise we would only have a &mut R.
impl<R> FutureResultTrait for Option<R>
where
    R: Send + 'static,
{
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
