use std::{
    any::Any,
    collections::{HashMap, hash_map::Entry},
    marker::PhantomData,
    mem,
};

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
    waiters: HashMap<WakeHandleId, WaiterState>,
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
    #[expect(unused)]
    pub fn get_ref<T: Any>(&mut self, h: WakeHandle<T>) -> Option<&T> {
        match self.waiters.get(&h.0) {
            Some(WaiterState::Done(x)) => Some(x.downcast_ref().unwrap()),
            _ => None,
        }
    }
    /// Gets the value returned by the waiter if it is done. This moves the value out.
    pub fn get<T: Any>(&mut self, h: WakeHandle<T>) -> Option<T> {
        match self.waiters.entry(h.0) {
            Entry::Occupied(entry) => match entry.get() {
                WaiterState::Waiting(_) => None,
                WaiterState::Done(_) => {
                    let WaiterState::Done(x) = entry.remove() else {
                        unreachable!()
                    };
                    Some(*x.downcast().unwrap())
                }
                WaiterState::Dummy => unreachable!(),
            },
            Entry::Vacant(_) => None,
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
            let mut waiter = mem::take(self.queue.waiters.get_mut(&handle).unwrap());
            waiter.maybe_wake(self);
            *self.queue.waiters.get_mut(&handle).unwrap() = waiter;
        }
    }
    pub fn enqueue_waiter<W: Waiter + 'static>(&mut self, mut w: W) -> WakeHandle<W::Output> {
        if w.is_ready(self) {
            let ret = w.wake(self);
            self.queue.set_already_resolved_handle(ret)
        } else {
            self.queue.enqueue_waiter(w)
        }
    }
}
