use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    marker::PhantomData,
    ops::{ControlFlow, FromResidual, Try},
};

use indexmap::IndexMap;
use rustorio::{Resource, ResourceType};

use crate::{
    GameState,
    crafting::IsBundle,
    machine::{Priority, Producer},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WakeHandleId(u32);

#[derive(Debug)]
pub struct WakeHandle<T> {
    pub id: WakeHandleId,
    phantom: PhantomData<T>,
}

impl<T> Clone for WakeHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for WakeHandle<T> {}

pub enum Poll<T> {
    /// Waiting for another waiter to complete; don't poll again until that other waiter is done.
    WaitingFor(WakeHandleId),
    /// This waiter is waiting for a resource; it will be updated directly by the resource
    /// producer.
    WaitingForResource,
    /// The waiter is done.
    Ready(T),
}

impl<T> FromResidual for Poll<T> {
    fn from_residual(residual: <Self as Try>::Residual) -> Self {
        match residual {
            Poll::Ready(x) => x,
            Poll::WaitingFor(h) => Poll::WaitingFor(h),
            Poll::WaitingForResource => Poll::WaitingForResource,
        }
    }
}
impl<T> Try for Poll<T> {
    type Output = T;
    type Residual = Poll<!>;
    fn from_output(output: Self::Output) -> Self {
        Poll::Ready(output)
    }
    fn branch(self) -> ControlFlow<Self::Residual, Self::Output> {
        match self {
            Poll::Ready(v) => ControlFlow::Continue(v),
            Poll::WaitingFor(h) => ControlFlow::Break(Poll::WaitingFor(h)),
            Poll::WaitingForResource => ControlFlow::Break(Poll::WaitingForResource),
        }
    }
}

pub trait Waiter {
    type Output: Any;
    fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output>;
}

trait UntypedWaiter {
    fn poll(&mut self, state: &mut GameState) -> Poll<Box<dyn Any>>;
}

struct Untype<W>(W);
impl<W: Waiter> UntypedWaiter for Untype<W> {
    fn poll(&mut self, state: &mut GameState) -> Poll<Box<dyn Any>> {
        match self.0.poll(state) {
            Poll::WaitingFor(h) => Poll::WaitingFor(h),
            Poll::WaitingForResource => Poll::WaitingForResource,
            Poll::Ready(val) => Poll::Ready(Box::new(val)),
        }
    }
}

struct WaiterState {
    waiter: Box<dyn UntypedWaiter>,
    /// Wake these other waiters up when done.
    dependent: Vec<WakeHandleId>,
}

#[derive(Default)]
pub struct WaiterQueue {
    next_handle: WakeHandleId,
    /// Waiters that should be polled.
    waiters: IndexMap<WakeHandleId, WaiterState>,
    /// Waiters waiting on some external event.
    dormant_waiters: IndexMap<WakeHandleId, WaiterState>,
    /// Return values of the waiters that have completed.
    done: HashMap<WakeHandleId, Box<dyn Any>>,
    /// Waiters in need of update, e.g. because the waiter they were waiting on got completed.
    needs_update: VecDeque<WakeHandleId>,
}

impl WaiterQueue {
    fn next_handle_id(&mut self) -> WakeHandleId {
        let h = self.next_handle;
        self.next_handle = WakeHandleId(h.0 + 1);
        h
    }
    /// Enqueues a waiter and returns a handle to wait for it.
    fn enqueue_waiter<W: Waiter + 'static>(&mut self, w: W) -> WakeHandle<W::Output> {
        let h = self.next_handle_id();
        self.waiters.insert(
            h,
            WaiterState {
                waiter: Box::new(Untype(w)),
                dependent: vec![],
            },
        );
        WakeHandle {
            id: h,
            phantom: PhantomData,
        }
    }
    /// Enqueues a fake waiter that's already done.
    pub fn set_already_resolved_handle<T: Any>(&mut self, x: T) -> WakeHandle<T> {
        let h = self.next_handle_id();
        self.done.insert(h, Box::new(x));
        WakeHandle {
            id: h,
            phantom: PhantomData,
        }
    }
    /// Enqueues a fake waiter that's never done.
    pub fn set_never_resolved_handle<T: Any>(&mut self) -> WakeHandle<T> {
        struct W<T>(PhantomData<T>);
        impl<T: Any> Waiter for W<T> {
            type Output = T;
            fn poll(&mut self, _state: &mut GameState) -> Poll<Self::Output> {
                Poll::WaitingForResource
            }
        }
        self.enqueue_waiter(W(PhantomData::<T>))
    }
    /// Set the output of this waiter. Its waiting code will no longer be run.
    pub fn set_output<T: Any>(&mut self, h: WakeHandle<T>, x: T) {
        let w = self
            .waiters
            .swap_remove(&h.id)
            .or_else(|| self.dormant_waiters.swap_remove(&h.id))
            .unwrap();
        self.needs_update.extend(w.dependent);
        self.done.insert(h.id, Box::new(x));
    }
    /// Gets the value returned by the waiter if it is done. This moves the value out.
    pub fn get<T: Any>(&mut self, h: WakeHandle<T>) -> Option<T> {
        Some(*self.done.remove(&h.id)?.downcast::<T>().unwrap())
    }
    /// Gets the value returned by the waiter if it is done. This does not move the value out.
    pub fn get_ref<T: Any>(&mut self, h: WakeHandle<T>) -> Option<&T> {
        Some(self.done.get(&h.id)?.downcast_ref::<T>().unwrap())
    }
    pub fn is_ready(&mut self, id: WakeHandleId) -> bool {
        self.done.contains_key(&id)
    }

    /// Enqueue all the waiting waiters into the update queue. Get them one by one with
    /// `next_waiter_to_update`. We can't poll them directly because we need full access to the
    /// `GameState`.
    pub fn enqueue_waiters_for_update(&mut self) {
        self.needs_update.extend(self.waiters.keys().copied())
    }
    /// Get the next waiter to update from the update queue.
    pub fn next_waiter_to_update(&mut self) -> Option<WakeHandleId> {
        self.needs_update.pop_front()
    }
}

impl GameState {
    pub fn enqueue_waiter<W: Waiter + 'static>(&mut self, mut w: W) -> WakeHandle<W::Output> {
        if let Poll::Ready(ret) = w.poll(self) {
            self.nowait(ret)
        } else {
            self.queue.enqueue_waiter(w)
        }
    }
    pub fn nowait<T: Any>(&mut self, x: T) -> WakeHandle<T> {
        self.queue.set_already_resolved_handle(x)
    }
    pub fn never<T: Any>(&mut self) -> WakeHandle<T> {
        self.queue.set_never_resolved_handle()
    }

    pub fn check_waiters(&mut self) {
        let mut scale_ups: Vec<Box<dyn FnOnce(&mut GameState)>> = vec![];
        for m in self.resources.producers() {
            m.update(&self.tick, &mut self.queue);
            if let Some(f) = m.scale_up_if_needed() {
                scale_ups.push(f);
            }
        }
        for f in scale_ups {
            f(self)
        }
        self.queue.enqueue_waiters_for_update();
        while let Some(handle) = self.queue.next_waiter_to_update() {
            self.check_waiter(handle);
        }
    }

    fn check_waiter(&mut self, handle: WakeHandleId) -> Option<()> {
        let mut w = self
            .queue
            .waiters
            .swap_remove(&handle)
            .or_else(|| self.queue.dormant_waiters.swap_remove(&handle))?;
        match w.waiter.poll(self) {
            Poll::Ready(val) => {
                self.queue.needs_update.extend(w.dependent);
                self.queue.done.insert(handle, val);
                return Some(());
            }
            Poll::WaitingFor(waiting_for) => {
                if let Some(waiting_for) = self
                    .queue
                    .waiters
                    .get_mut(&waiting_for)
                    .or_else(|| self.queue.dormant_waiters.get_mut(&waiting_for))
                {
                    waiting_for.dependent.push(handle);
                    self.queue.dormant_waiters.insert(handle, w);
                } else {
                    self.queue.waiters.insert(handle, w);
                }
            }
            Poll::WaitingForResource => {
                self.queue.dormant_waiters.insert(handle, w);
            }
        }
        Some(())
    }

    /// Poll that waiter to get its value if it is ready.
    fn poll_waiter<T: Any>(&mut self, handle: WakeHandle<T>) -> Poll<T> {
        if let Some(v) = self.queue.get(handle) {
            Poll::Ready(v)
        } else {
            Poll::WaitingFor(handle.id)
        }
    }
    pub fn is_ready(&mut self, id: WakeHandleId) -> bool {
        self.queue.is_ready(id)
    }

    pub fn map<T: Any, U: Any>(
        &mut self,
        h: WakeHandle<T>,
        f: impl FnOnce(&mut GameState, T) -> U + 'static,
    ) -> WakeHandle<U> {
        struct W<F, T>(WakeHandle<T>, Option<F>);
        impl<F, T: Any, U: Any> Waiter for W<F, T>
        where
            F: FnOnce(&mut GameState, T) -> U,
        {
            type Output = U;
            fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output> {
                let v = state.poll_waiter(self.0)?;
                let f = self.1.take().unwrap();
                let v = f(state, v);
                Poll::Ready(v)
            }
        }
        self.enqueue_waiter(W(h, Some(f)))
    }

    /// Schedules `f` to run after `h` completes, and returns a hendl to the final output.
    pub fn then<T: Any, U: Any>(
        &mut self,
        h: WakeHandle<T>,
        f: impl FnOnce(&mut GameState, T) -> WakeHandle<U> + 'static,
    ) -> WakeHandle<U> {
        enum W<F, T, U> {
            First(WakeHandle<T>, Option<F>),
            Second(WakeHandle<U>),
        }
        impl<F, T: Any, U: Any> Waiter for W<F, T, U>
        where
            F: FnOnce(&mut GameState, T) -> WakeHandle<U>,
        {
            type Output = U;
            fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output> {
                if let W::First(h, f) = self {
                    let v = state.poll_waiter(*h)?;
                    let f = f.take().unwrap();
                    *self = W::Second(f(state, v));
                }
                let W::Second(h) = self else { unreachable!() };
                let v = state.poll_waiter(*h)?;
                Poll::Ready(v)
            }
        }
        self.enqueue_waiter(W::First(h, Some(f)))
    }

    /// Joins the results of two handles together.
    pub fn pair<T: Any, U: Any>(
        &mut self,
        x: WakeHandle<T>,
        y: WakeHandle<U>,
    ) -> WakeHandle<(T, U)> {
        struct W<T, U>(WakeHandle<T>, WakeHandle<U>);
        impl<T: Any, U: Any> Waiter for W<T, U> {
            type Output = (T, U);
            fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output> {
                let Self(x, y) = *self;
                if let Some(id) = [x.id, y.id]
                    .into_iter()
                    .filter(|&h| !state.is_ready(h))
                    .last()
                {
                    return Poll::WaitingFor(id);
                }
                let v = (state.queue.get(x).unwrap(), state.queue.get(y).unwrap());
                Poll::Ready(v)
            }
        }
        self.enqueue_waiter(W(x, y))
    }

    pub fn triple<T: Any, U: Any, V: Any>(
        &mut self,
        x: WakeHandle<T>,
        y: WakeHandle<U>,
        z: WakeHandle<V>,
    ) -> WakeHandle<(T, U, V)> {
        let xy = self.pair(x, y);
        let xyz = self.pair(xy, z);
        self.map(xyz, |_, ((x, y), z)| (x, y, z))
    }

    /// Joins the results of two handles together.
    pub fn collect<T: Any>(&mut self, handles: Vec<WakeHandle<T>>) -> WakeHandle<Vec<T>> {
        struct W<T>(Vec<WakeHandle<T>>);
        impl<T: Any> Waiter for W<T> {
            type Output = Vec<T>;
            fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output> {
                if let Some(h) = self.0.iter().filter(|&&h| !state.is_ready(h.id)).last() {
                    return Poll::WaitingFor(h.id);
                }
                let v = self
                    .0
                    .iter()
                    .map(|h| state.queue.get(*h).unwrap())
                    .collect();
                Poll::Ready(v)
            }
        }
        self.enqueue_waiter(W(handles))
    }

    /// Waits for the selected producer to produce a single output bundle.
    /// This is the main wait point of our system.
    pub fn wait_for_producer_output<P: Producer>(&mut self, p: Priority) -> WakeHandle<P::Output> {
        struct W<P>(PhantomData<P>);
        impl<P: Producer> Waiter for W<P> {
            type Output = P::Output;
            fn poll(&mut self, _state: &mut GameState) -> Poll<Self::Output> {
                Poll::WaitingForResource
            }
        }
        let h = self.queue.enqueue_waiter(W(PhantomData::<P>));
        P::get_ref(&mut self.resources).enqueue(&self.tick, &mut self.queue, h, p);
        h
    }

    pub fn collect_sum<R: ResourceType + Any, B: IsBundle<Resource = R> + Any>(
        &mut self,
        handles: Vec<WakeHandle<B>>,
    ) -> WakeHandle<Resource<R>> {
        let h = self.collect(handles);
        self.map(h, |_state, bundles| {
            bundles.into_iter().map(|b| b.to_resource()).sum()
        })
    }
}
