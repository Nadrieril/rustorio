use std::{
    any::Any,
    collections::VecDeque,
    marker::PhantomData,
    mem,
    ops::{ControlFlow, FromResidual, Try},
};

use indexmap::{IndexMap, map::Entry};
use rustorio::{Bundle, Resource, ResourceType};

use crate::{
    GameState, Resources,
    crafting::{ConstRecipe, Makeable},
    machine::{Machine, MachineSlot, Producer, ProducerWithQueue},
};

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
    /// Waiting for an unspecified reason.
    Pending,
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
            Poll::Pending => Poll::Pending,
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
            Poll::Pending => ControlFlow::Break(Poll::Pending),
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
            Poll::Pending => Poll::Pending,
            Poll::WaitingFor(h) => Poll::WaitingFor(h),
            Poll::WaitingForResource => Poll::WaitingForResource,
            Poll::Ready(val) => Poll::Ready(Box::new(val)),
        }
    }
}

#[derive(Default)]
enum WaiterState {
    Waiting {
        waiter: Box<dyn UntypedWaiter>,
        /// Wake these other waiters up when done.
        dependent: Vec<WakeHandleId>,
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
    /// Waiters in need of update, e.g. because the waiter they were waiting on got completed.
    needs_update: VecDeque<WakeHandleId>,
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
        self.enqueue_waiter_with(|_| w)
    }
    /// Enqueues a waiter and returns a handle to wait for it.
    pub fn enqueue_waiter_with<W: Waiter + 'static>(
        &mut self,
        f: impl FnOnce(WakeHandleId) -> W,
    ) -> WakeHandle<W::Output> {
        let h = self.next_handle_id();
        self.waiters.insert(
            h,
            WaiterState::Waiting {
                waiter: Box::new(Untype(f(h))),
                dependent: vec![],
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
    /// Set the output of this waiter. Its waiting code will no longer be run.
    pub fn set_output<T: Any>(&mut self, h: WakeHandle<T>, x: T) {
        let w = self.waiters.get_mut(&h.id).unwrap();
        if let WaiterState::Waiting { dependent, .. } = w {
            self.needs_update.extend(dependent.drain(..));
        }
        *w = WaiterState::Done(Box::new(x));
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

    /// Enqueue all the waiting waiters into the update queue. Get them one by one with
    /// `next_waiter_to_update`. We can't poll them directly because we need full access to the
    /// `GameState`.
    pub fn enqueue_waiters_for_update(&mut self) {
        self.needs_update.extend(
            self.waiters
                .iter()
                .filter(|(_, waiter)| matches!(waiter, WaiterState::Waiting { dormant: false, .. }))
                .map(|(h, _)| *h),
        )
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

    pub fn check_waiters(&mut self) {
        self.resources
            .iron_territory
            .as_mut()
            .unwrap()
            .update(&self.tick, &mut self.queue);
        self.resources
            .copper_territory
            .as_mut()
            .unwrap()
            .update(&self.tick, &mut self.queue);
        self.queue.enqueue_waiters_for_update();
        while let Some(handle) = self.queue.next_waiter_to_update() {
            self.check_waiter(handle);
        }
    }
    fn check_waiter(&mut self, handle: WakeHandleId) -> Option<()> {
        let mut w = mem::take(self.queue.waiters.get_mut(&handle)?);
        if let WaiterState::Waiting {
            waiter,
            dormant,
            dependent,
        } = &mut w
        {
            match waiter.poll(self) {
                Poll::Ready(val) => {
                    self.queue.needs_update.extend(dependent.drain(..));
                    w = WaiterState::Done(val);
                }
                Poll::Pending => {
                    *dormant = false; // in case we just polled a waiter that was dormant.
                }
                Poll::WaitingFor(waiting_for) => {
                    if let Some(WaiterState::Waiting { dependent: dep, .. }) =
                        self.queue.waiters.get_mut(&waiting_for)
                    {
                        dep.push(handle);
                        *dormant = true;
                    }
                }
                Poll::WaitingForResource => {
                    *dormant = true;
                }
            }
        }
        *self.queue.waiters.get_mut(&handle).unwrap() = w;
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

    /// Waits for the selected machine to produce a single output bundle.
    /// This is the main wait point of our system.
    pub fn wait_for_machine_output<M>(
        &mut self,
        slot: MachineSlot<M>,
    ) -> WakeHandle<<M::Recipe as ConstRecipe>::BundledOutputs>
    where
        M: Machine + Makeable,
        M::Recipe: ConstRecipe + Any,
    {
        struct W<M>(MachineSlot<M>, WakeHandleId);

        impl<M> Waiter for W<M>
        where
            M: Machine + Makeable,
            M::Recipe: ConstRecipe + Any,
        {
            type Output = <M::Recipe as ConstRecipe>::BundledOutputs;
            fn poll(&mut self, state: &mut GameState) -> Poll<Self::Output> {
                let machine = state.resources.machine_store.get_slot(self.0);
                match machine.get_outputs(&state.tick) {
                    Some(x) => {
                        // Remove ourselves from the queue.
                        machine.queue.retain(|h| *h != self.1);
                        Poll::Ready(x)
                    }
                    None => {
                        // Declare that we're waiting on whoever's before.
                        let ret = if let Some(&last) = machine.queue.front()
                            && last != self.1
                        {
                            // Poll::WaitingFor(last)
                            Poll::Pending
                        } else {
                            Poll::Pending
                        };
                        // Add ourselves to the queue.
                        if !machine.queue.contains(&self.1) {
                            machine.queue.push_back(self.1);
                        }
                        // TODO: Doesn't particularly make sense to wait on a specific slot. Could
                        // be that for each resource producer we have a queue. Each tick we poll
                        // all the producers, move their output to the resource stores, and wake
                        // each waiter until one returns NeedsMoreResource.
                        // The one thing we promise is that a waiter can be enqueued only if it
                        // provided the right inputs.
                        // This means we know exactly the load on each producer, and can scale
                        // appropriately.
                        // Ideal is having n waitees for n producers. What's a load criterion for
                        // making more producers?
                        // Maybe we want the derivative: if the load is increasing over time we
                        // should try to stabilize it. Beware loops: copper wire is needed to scale
                        // up copper wire production. Should probably not be building several new
                        // assemblers of a given type at a given moment.
                        // We do need to load balace the inputs? Unless work stealing: each tick if
                        // a producer would not have enough inputs for the next output it steals
                        // one from the common pool. Important to use per-type input pools I think,
                        // to avoid the topology getting messed up. Annoying to do generically
                        // compared to round-robin assignment.
                        //
                        // In the grand scheme of things, I should distribute output to the waiters
                        // who'll be able to make progress. The good thing is that we're getting
                        // closer to defunctionalization: can have an enum of all the possible
                        // waiters and possibly compute clever things. Ideally we'd compute the
                        // full dep graph from the start. Question 1 is ordering of who gets what
                        // first, question 2 is that scaling up changes the graph.
                        //
                        // Depending on load I can choose at each tick what to handcraft too! fun,
                        // though loses time because each tick counts with the granularity I use.
                        // What local decisions can I even make...
                        //
                        // For a given resource type we can switch producers: while there is no
                        // assembler we handcraft, then switch to assembler. Seems to be able to
                        // cover territories too.
                        ret
                    }
                }
            }
        }
        match self
            .resources
            .machine_store
            .get_slot(slot)
            .get_outputs(&self.tick)
        {
            Some(x) => self.nowait(x),
            None => self.queue.enqueue_waiter_with(|id| W(slot, id)),
        }
    }

    /// Waits for the selected producer to produce a single output bundle.
    /// This is the main wait point of our system.
    pub fn wait_for_producer_output<P: Producer>(
        &mut self,
        producer: fn(&mut Resources) -> &mut ProducerWithQueue<P>,
    ) -> WakeHandle<P::Output> {
        struct W<P>(PhantomData<P>);
        impl<P: Producer> Waiter for W<P> {
            type Output = P::Output;
            fn poll(&mut self, _state: &mut GameState) -> Poll<Self::Output> {
                Poll::WaitingForResource
            }
        }
        let h = self.queue.enqueue_waiter(W(PhantomData::<P>));
        producer(&mut self.resources).enqueue(&self.tick, &mut self.queue, h);
        h
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
}
