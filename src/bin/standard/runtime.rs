use std::{
    any::Any,
    ops::{ControlFlow, Deref},
};

use itertools::Itertools;
use rustorio::Tick;

use crate::{
    Resources, StartingResources,
    machine::AdvancedTick,
    scheduler::{WaiterQueue, WakeHandle},
};

/// Wrapper to restrict mutable access.
pub struct RestrictMut<T>(T);

/// Must only be created when a tick is explicitly requested. Jobs which require mutable access to
/// the `Tick` should enqueue themselves to `GameState.mut_tick_queue`.
pub struct RestrictMutToken(());

impl<T> RestrictMut<T> {
    fn new(x: T) -> Self {
        Self(x)
    }
    fn as_ref(&self) -> &T {
        &self.0
    }
    pub fn as_mut(&mut self, _: RestrictMutToken) -> &mut T {
        &mut self.0
    }
}
impl<T> Deref for RestrictMut<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

pub struct GameState {
    pub tick: RestrictMut<Tick>,
    last_reported_tick: u64,
    pub resources: Resources,
    pub queue: WaiterQueue,
}

impl GameState {
    pub fn new(mut tick: Tick, starting_resources: StartingResources) -> Self {
        tick.log(false);
        GameState {
            tick: RestrictMut::new(tick),
            last_reported_tick: 0,
            queue: Default::default(),
            resources: Resources::new(starting_resources),
        }
    }

    pub fn tick(&self) -> &Tick {
        &self.tick
    }
    pub fn tick_fwd(&mut self) {
        let mut_token = RestrictMutToken(()); // Only place where we create one.

        match self.resources.with_hand_producers(|p| {
            p.craft_by_hand_if_needed(self.tick.as_mut(RestrictMutToken(())))
        }) {
            ControlFlow::Break(AdvancedTick) => {}
            ControlFlow::Continue(()) => self.tick.as_mut(mut_token).advance(),
        }

        self.check_waiters();
        self.report_loads();
    }

    pub fn report_loads(&mut self) {
        if self.tick.cur() / 50 == self.last_reported_tick / 50 {
            return;
        }
        self.last_reported_tick = self.tick.cur();

        // TODO: how about add a machine if load too big? Clients are only waiting when the inputs
        // are ready so this isn't backpressure, it's a bottleneck.
        let r = &mut self.resources;
        let loads = r
            .producers()
            .map(|p| (p.name(), p.load()))
            .sorted()
            .map(|(name, load)| format!(" - {name}: {load}\n"))
            .format("");
        eprintln!("{}:\n{}", self.tick.as_ref(), loads)
    }

    pub fn complete<R: Any>(&mut self, h: WakeHandle<R>) -> R {
        loop {
            if let Some(ret) = self.queue.get(h) {
                return ret;
            }
            self.tick_fwd();
        }
    }
    pub fn complete_all(&mut self) {
        loop {
            if self.queue.is_all_done() {
                println!("Completed all in {} ticks", self.tick().cur());
                return;
            }
            self.tick_fwd();
        }
    }
}
