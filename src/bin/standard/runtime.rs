use std::{
    any::Any,
    ops::{ControlFlow, Deref},
};

use itertools::Itertools;
use rustorio::Tick;

use crate::{
    Resources, StartingResources,
    machine::AdvancedTick,
    scheduler::{CallBackQueue, WakeHandle},
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
    pub fn into_inner(self) -> T {
        self.0
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
    pub queue: CallBackQueue,
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

        let tick_mut = self.tick.as_mut(mut_token);
        match self
            .resources
            .with_hand_producers(|p| p.craft_by_hand_if_needed(tick_mut))
        {
            ControlFlow::Break(AdvancedTick) => {}
            ControlFlow::Continue(()) => tick_mut.advance(),
        }

        self.check_waiters();
        self.report_loads();
    }

    pub fn check_waiters(&mut self) {
        let mut scale_ups = vec![];
        for m in self.resources.iter_producers() {
            m.update(&self.tick, &mut self.queue);
            if let Some(f) = m.scale_up_if_needed() {
                scale_ups.push(f);
            }
        }
        for f in scale_ups {
            f(self);
        }
        while let Some(f) = self.queue.next_callback() {
            f(self);
        }
    }

    pub fn report_loads(&mut self) {
        // const REPORT_PERIOD: u64 = 5;
        const REPORT_PERIOD: u64 = 100;
        if self.tick.cur() / REPORT_PERIOD == self.last_reported_tick / REPORT_PERIOD {
            return;
        }
        self.last_reported_tick = self.tick.cur();

        // TODO: how about add a machine if load too big? Clients are only waiting when the inputs
        // are ready so this isn't backpressure, it's a bottleneck.
        let r = &mut self.resources;
        let loads = r
            .iter_producers()
            .sorted_by_key(|p| p.name())
            .map(|p| {
                format!(
                    " - {} (x{}): {}\n",
                    p.name(),
                    p.available_parallelism(),
                    p.report_load(&self.tick)
                )
            })
            .format("");
        eprintln!("{}:\n{}", self.tick.as_ref(), loads);
    }

    pub fn complete<R: Any>(&mut self, mut h: WakeHandle<R>) -> R {
        let ControlFlow::Break(ret) = try {
            loop {
                h = h.try_get()?;
                self.tick_fwd();
                if self.tick.cur() > 10000 {
                    panic!("ticked too far?")
                }
            }
        };
        ret
    }
}
