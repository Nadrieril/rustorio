use std::{any::Any, marker::PhantomData, mem};

use indexmap::{IndexMap, map::Entry};
use itertools::Itertools;

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
    },
    Done(Box<dyn Any>),
    #[default]
    BeingChecked, // Dummy state for mem::replace
}

impl WaiterState {
    pub fn maybe_wake(&mut self, state: &mut GameState) {
        if let WaiterState::Waiting { waiter, .. } = self
            && let Poll::Ready(val) = waiter.poll(state)
        {
            *self = WaiterState::Done(val);
        }
    }
}

#[derive(Default)]
pub struct WaiterQueue {
    next_handle: WakeHandleId,
    waiters: IndexMap<WakeHandleId, WaiterState>,
    /// Waiters that are waiting on another one.
    dormant: IndexMap<WakeHandleId, WaiterState>,
}

impl WaiterQueue {
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
    pub fn is_ready<T: Any>(&mut self, h: WakeHandle<T>) -> bool {
        match self.waiters.get(&h.id) {
            Some(WaiterState::Done(_)) => true,
            _ => false,
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
            Entry::Vacant(_) => panic!("the value has been taken already"),
        }
    }

    /// Go through the waiters and find those that are ready to be checked up. The waiters should
    /// be woken up with `WaiterState::wake` directly because with a method we'd have to separate
    /// the GameState from the queue but the waking up may need to enqueue new things.
    pub fn waiters_to_check(&self) -> Vec<WakeHandleId> {
        self.waiters
            .iter()
            .filter(|(_, waiter)| matches!(waiter, WaiterState::Waiting { .. }))
            .map(|(h, _)| *h)
            .collect_vec()
    }
}

impl GameState {
    pub fn check_waiters(&mut self) {
        for handle in self.queue.waiters_to_check() {
            self.check_waiter(handle);
        }
    }
    fn check_waiter(&mut self, handle: WakeHandleId) -> Option<()> {
        let mut waiter = mem::take(self.queue.waiters.get_mut(&handle)?);
        waiter.maybe_wake(self);
        *self.queue.waiters.get_mut(&handle).unwrap() = waiter;
        Some(())
    }
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
                let _ = state.check_waiter(self.0.id);
                if state.queue.is_ready(self.0) {
                    let v = state.queue.get(self.0).unwrap();
                    let f = self.1.take().unwrap();
                    let v = f(state, v);
                    Poll::Ready(v)
                } else {
                    Poll::WaitingFor(self.0.id)
                }
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
                    let _ = state.check_waiter(h.id);
                    let Some(v) = state.queue.get(*h) else {
                        return Poll::WaitingFor(h.id);
                    };
                    let f = mem::take(f).unwrap();
                    *self = W::Second(f(state, v));
                }
                let W::Second(h) = self else { unreachable!() };
                state.check_waiter(h.id);
                let Some(v) = state.queue.get(*h) else {
                    return Poll::WaitingFor(h.id);
                };
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
                let _ = state.check_waiter(x.id);
                if !state.queue.is_ready(x) {
                    return Poll::WaitingFor(x.id);
                }
                let _ = state.check_waiter(y.id);
                if !state.queue.is_ready(y) {
                    return Poll::WaitingFor(y.id);
                }
                let Self(x, y) = *self;
                let v = (state.queue.get(x).unwrap(), state.queue.get(y).unwrap());
                Poll::Ready(v)
            }
        }
        self.enqueue_waiter(W(x, y))
    }

    #[expect(unused)]
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
                for h in self.0.iter() {
                    let _ = state.check_waiter(h.id);
                    if !state.queue.is_ready(*h) {
                        return Poll::WaitingFor(h.id);
                    }
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
}
