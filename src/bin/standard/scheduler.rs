use std::{
    any::Any,
    marker::PhantomData,
    mem,
    ops::{ControlFlow, FromResidual, Try},
};

use indexmap::{IndexMap, map::Entry};
use itertools::Itertools;
use rustorio::{Bundle, Resource, ResourceType};

use crate::GameState;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WakeHandleId(u32);

#[derive(Debug)]
pub struct WakeHandle<T> {
    id: WakeHandleId,
    phantom: PhantomData<T>,
}

impl<T> Clone for WakeHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for WakeHandle<T> {}

pub enum Poll<T> {
    Pending,
    WaitingFor(WakeHandleId),
    Ready(T),
}

impl<T> FromResidual for Poll<T> {
    fn from_residual(residual: <Self as Try>::Residual) -> Self {
        match residual {
            Poll::Ready(x) => x,
            Poll::Pending => Poll::Pending,
            Poll::WaitingFor(h) => Poll::WaitingFor(h),
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
            Poll::Pending => ControlFlow::Break(Poll::Pending),
            Poll::WaitingFor(h) => ControlFlow::Break(Poll::WaitingFor(h)),
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
            Poll::Pending => Poll::Pending,
            Poll::WaitingFor(h) => Poll::WaitingFor(h),
            Poll::Ready(val) => Poll::Ready(Box::new(val)),
        }
    }
}

#[derive(Default)]
enum WaiterState {
    Waiting {
        waiter: Box<dyn UntypedWaiter>,
        /// Wake this other waiter up when done.
        dependent: Option<WakeHandleId>,
        /// Waiters that are waiting on another one.
        dormant: bool,
    },
    Done(Box<dyn Any>),
    #[default]
    BeingChecked, // Dummy state for mem::replace
}

#[derive(Default)]
pub struct WaiterQueue {
    next_handle: WakeHandleId,
    waiters: IndexMap<WakeHandleId, WaiterState>,
}

impl WaiterQueue {
    pub fn is_all_done(&self) -> bool {
        self.waiters
            .iter()
            .all(|(_, w)| matches!(w, WaiterState::Done(..)))
    }
    fn next_handle_id(&mut self) -> WakeHandleId {
        let h = self.next_handle;
        self.next_handle = WakeHandleId(h.0 + 1);
        h
    }
    /// Enqueues a waiter and returns a handle to wait for it.
    pub fn enqueue_waiter<W: Waiter + 'static>(&mut self, w: W) -> WakeHandle<W::Output> {
        let h = self.next_handle_id();
        self.waiters.insert(
            h,
            WaiterState::Waiting {
                waiter: Box::new(Untype(w)),
                dependent: None,
                dormant: false,
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
        self.waiters.insert(h, WaiterState::Done(Box::new(x)));
        WakeHandle {
            id: h,
            phantom: PhantomData,
        }
    }
    /// Gets the value returned by the waiter if it is done. This moves the value out.
    pub fn get<T: Any>(&mut self, h: WakeHandle<T>) -> Option<T> {
        match self.waiters.entry(h.id) {
            Entry::Occupied(entry) => match entry.get() {
                WaiterState::Waiting { .. } => None,
                WaiterState::Done(_) => {
                    let WaiterState::Done(x) = entry.shift_remove() else {
                        unreachable!()
                    };
                    Some(*x.downcast().unwrap())
                }
                WaiterState::BeingChecked => unreachable!(),
            },
            Entry::Vacant(_) => None,
        }
    }
    /// Gets the value returned by the waiter if it is done. This does not move the value out.
    pub fn get_ref<T: Any>(&mut self, h: WakeHandle<T>) -> Option<&T> {
        match self.waiters.get(&h.id) {
            Some(WaiterState::Done(x)) => Some(x.downcast_ref().unwrap()),
            _ => None,
        }
    }
    pub fn is_ready(&mut self, id: WakeHandleId) -> bool {
        match self.waiters.get(&id) {
            Some(WaiterState::Done(_)) => true,
            _ => false,
        }
    }

    /// Go through the waiters and find those that are ready to be checked up. The waiters should
    /// be woken up with `WaiterState::wake` directly because with a method we'd have to separate
    /// the GameState from the queue but the waking up may need to enqueue new things.
    pub fn waiters_to_check(&self) -> Vec<WakeHandleId> {
        self.waiters
            .iter()
            .filter(|(_, waiter)| matches!(waiter, WaiterState::Waiting { dormant: false, .. }))
            .map(|(h, _)| *h)
            .collect_vec()
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

    pub fn check_waiters(&mut self) {
        for handle in self.queue.waiters_to_check() {
            self.check_waiter(handle);
        }
    }
    fn check_waiter(&mut self, handle: WakeHandleId) -> Option<()> {
        let mut w = mem::take(self.queue.waiters.get_mut(&handle)?);
        let mut check_after = None;
        if let WaiterState::Waiting {
            ref mut waiter,
            ref mut dormant,
            dependent,
        } = w
        {
            match waiter.poll(self) {
                Poll::Ready(val) => {
                    check_after = dependent;
                    w = WaiterState::Done(val);
                }
                Poll::Pending => {
                    *dormant = false; // in case we just polled a waiter that was dormant.
                }
                Poll::WaitingFor(waiting_for) => {
                    if let Some(WaiterState::Waiting {
                        dependent: dep @ None,
                        ..
                    }) = self.queue.waiters.get_mut(&waiting_for)
                    {
                        *dep = Some(handle);
                        *dormant = true;
                    }
                }
            }
        }
        *self.queue.waiters.get_mut(&handle).unwrap() = w;
        if let Some(deph) = check_after {
            self.check_waiter(deph);
        }
        Some(())
    }
    /// Poll that waiter to get its value if it is ready.
    fn poll_waiter<T: Any>(&mut self, handle: WakeHandle<T>) -> Poll<T> {
        let _ = self.check_waiter(handle.id);
        if let Some(v) = self.queue.get(handle) {
            Poll::Ready(v)
        } else {
            Poll::WaitingFor(handle.id)
        }
    }
    /// Poll waiter without moving the value out.
    #[expect(unused)]
    fn poll_waiter_ref<T: Any>(&mut self, handle: WakeHandle<T>) -> Poll<&T> {
        let _ = self.check_waiter(handle.id);
        if let Some(v) = self.queue.get_ref(handle) {
            Poll::Ready(v)
        } else {
            Poll::WaitingFor(handle.id)
        }
    }
    pub fn is_ready(&mut self, id: WakeHandleId) -> bool {
        let _ = self.check_waiter(id);
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

    /// Waits until the function returns `Some` and yields the returned value.
    pub fn wait_for<T: Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> Option<T> + 'static,
    ) -> WakeHandle<T> {
        struct W<F>(F);

        impl<F, T: Any> Waiter for W<F>
        where
            F: Fn(&mut GameState) -> Option<T>,
        {
            type Output = T;
            fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output> {
                match (self.0)(state) {
                    Some(x) => Poll::Ready(x),
                    None => Poll::Pending,
                }
            }
        }
        self.enqueue_waiter(W(f))
    }

    pub fn collect_sum<const COUNT: u32, R: ResourceType + Any>(
        &mut self,
        handles: Vec<WakeHandle<Bundle<R, COUNT>>>,
    ) -> WakeHandle<Resource<R>> {
        let h = self.collect(handles);
        self.map(h, |_state, bundles| {
            bundles.into_iter().map(|b| b.to_resource()).sum()
        })
    }

    /// Waits until the function returns `true`.
    #[expect(unused)]
    pub fn wait_until(&mut self, f: impl Fn(&mut GameState) -> bool + 'static) -> WakeHandle<()> {
        self.wait_for(move |state| f(state).then_some(()))
    }
}
