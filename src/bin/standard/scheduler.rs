use std::{any::Any, marker::PhantomData, mem};

use indexmap::{IndexMap, map::Entry};
use itertools::Itertools;

use crate::GameState;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WakeHandleId(u32);

#[derive(Debug)]
pub struct WakeHandle<T>(WakeHandleId, PhantomData<T>);

impl<T> Clone for WakeHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for WakeHandle<T> {}

pub trait Waiter {
    type Output: Any;
    fn is_ready(&mut self, state: &mut GameState) -> bool;
    fn wake(self, state: &mut GameState) -> Self::Output;
}

trait UntypedWaiter {
    fn is_ready(&mut self, state: &mut GameState) -> bool;
    fn wake(self: Box<Self>, state: &mut GameState) -> Box<dyn Any>;
}

struct Untype<W>(W);
impl<W: Waiter> UntypedWaiter for Untype<W> {
    fn is_ready(&mut self, state: &mut GameState) -> bool {
        self.0.is_ready(state)
    }
    fn wake(self: Box<Self>, state: &mut GameState) -> Box<dyn Any> {
        Box::new(self.0.wake(state))
    }
}

#[derive(Default)]
enum WaiterState {
    Waiting(Box<dyn UntypedWaiter>),
    Done(Box<dyn Any>),
    #[default]
    Dummy, // Dummy state for mem::replace
}

impl WaiterState {
    // fn should_wake(&self, state: &mut GameState) -> bool {
    //     match self {
    //         WaiterState::Waiting(waiter) => waiter.is_ready(state),
    //         WaiterState::Done(_) => false,
    //         WaiterState::Dummy => unreachable!(),
    //     }
    // }
    pub fn maybe_wake(&mut self, state: &mut GameState) {
        if let WaiterState::Waiting(waiter) = self
            && waiter.is_ready(state)
        {
            let WaiterState::Waiting(waiter) = mem::replace(self, Self::Dummy) else {
                unreachable!()
            };
            let val = waiter.wake(state);
            *self = WaiterState::Done(val);
        }
    }
}

#[derive(Default)]
pub struct WaiterQueue {
    next_handle: WakeHandleId,
    waiters: IndexMap<WakeHandleId, WaiterState>,
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
        self.waiters
            .insert(h, WaiterState::Waiting(Box::new(Untype(w))));
        WakeHandle(h, PhantomData)
    }
    /// Enqueues a fake waiter that's already done.
    pub fn set_already_resolved_handle<T: Any>(&mut self, x: T) -> WakeHandle<T> {
        let h = self.next_handle_id();
        self.waiters.insert(h, WaiterState::Done(Box::new(x)));
        WakeHandle(h, PhantomData)
    }
    /// Get the value returned by the waiter if it is done.
    pub fn get_ref<T: Any>(&mut self, h: WakeHandle<T>) -> Option<&T> {
        match self.waiters.get(&h.0) {
            Some(WaiterState::Done(x)) => Some(x.downcast_ref().unwrap()),
            _ => None,
        }
    }
    pub fn is_ready<T: Any>(&mut self, h: WakeHandle<T>) -> bool {
        match self.waiters.get(&h.0) {
            Some(WaiterState::Done(_)) => true,
            _ => false,
        }
    }
    /// Gets the value returned by the waiter if it is done. This moves the value out.
    pub fn get<T: Any>(&mut self, h: WakeHandle<T>) -> Option<T> {
        match self.waiters.entry(h.0) {
            Entry::Occupied(entry) => match entry.get() {
                WaiterState::Waiting(_) => None,
                WaiterState::Done(_) => {
                    let WaiterState::Done(x) = entry.shift_remove() else {
                        unreachable!()
                    };
                    Some(*x.downcast().unwrap())
                }
                WaiterState::Dummy => unreachable!(),
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
            .filter(|(_, waiter)| matches!(waiter, WaiterState::Waiting(..)))
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
        if w.is_ready(self) {
            let ret = w.wake(self);
            self.queue.set_already_resolved_handle(ret)
        } else {
            self.queue.enqueue_waiter(w)
        }
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
            fn is_ready(&mut self, state: &mut GameState) -> bool {
                if let W::First(h, f) = self
                    && let _ = state.check_waiter(h.0)
                    && let Some(v) = state.queue.get(*h)
                {
                    let f = mem::take(f).unwrap();
                    *self = W::Second(f(state, v));
                }
                if let W::Second(h) = self {
                    state.check_waiter(h.0);
                    state.queue.get_ref(*h).is_some()
                } else {
                    false
                }
            }
            fn wake(self, state: &mut GameState) -> U {
                let W::Second(h) = self else { unreachable!() };
                state.queue.get(h).unwrap()
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
            fn is_ready(&mut self, state: &mut GameState) -> bool {
                let Self(x, y) = *self;
                let _ = state.check_waiter(x.0);
                let _ = state.check_waiter(y.0);
                state.queue.is_ready(x) && state.queue.is_ready(y)
            }
            fn wake(self, state: &mut GameState) -> (T, U) {
                let Self(x, y) = self;
                (state.queue.get(x).unwrap(), state.queue.get(y).unwrap())
            }
        }
        self.enqueue_waiter(W(x, y))
    }

    /// Joins the results of two handles together.
    pub fn join<T: Any>(&mut self, handles: Vec<WakeHandle<T>>) -> WakeHandle<Vec<T>> {
        struct W<T>(Vec<WakeHandle<T>>);
        impl<T: Any> Waiter for W<T> {
            type Output = Vec<T>;
            fn is_ready(&mut self, state: &mut GameState) -> bool {
                let mut all_ready = true;
                for h in self.0.iter() {
                    let _ = state.check_waiter(h.0);
                    if !state.queue.is_ready(*h) {
                        all_ready = false
                    }
                }
                all_ready
            }
            fn wake(self, state: &mut GameState) -> Vec<T> {
                self.0
                    .into_iter()
                    .map(|h| state.queue.get(h).unwrap())
                    .collect()
            }
        }
        self.enqueue_waiter(W(handles))
    }
}
