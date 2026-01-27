use std::{any::Any, collections::VecDeque, ops::Deref};

use itertools::Itertools;
use rustorio::Tick;

use crate::{
    Resources, StartingResources,
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
    /// Advancing time during the waiter updates risks skipping updates. So instead we require that
    /// jobs which need mutable ownership of the `Tick` be put in a separate queue.
    mut_tick_queue: VecDeque<Box<dyn FnOnce(&mut GameState, RestrictMutToken)>>,
    pub resources: Resources,
    pub queue: WaiterQueue,
}

impl GameState {
    pub fn new(mut tick: Tick, starting_resources: StartingResources) -> Self {
        tick.log(false);
        GameState {
            tick: RestrictMut::new(tick),
            last_reported_tick: 0,
            mut_tick_queue: Default::default(),
            queue: Default::default(),
            resources: Resources::new(starting_resources),
        }
    }

    pub fn tick(&self) -> &Tick {
        &self.tick
    }
    /// Enqueue an operation that requires advancing the tick time. It will be executed inside
    /// `tick_fwd` instead of plainly advancing the tick.
    pub fn with_mut_tick(&mut self, f: impl FnOnce(&mut GameState, RestrictMutToken) + Any) {
        self.mut_tick_queue.push_back(Box::new(f))
    }
    pub fn tick_fwd(&mut self) {
        let mut_token = RestrictMutToken(()); // Only place where we create one.

        let iron_territory = self.resources.iron_territory.as_mut().unwrap();
        let copper_territory = self.resources.copper_territory.as_mut().unwrap();
        if iron_territory.craft_by_hand_if_needed(self.tick.as_mut(RestrictMutToken(()))) {
            println!("handcrafting iron ore");
            // The tick has advanced
        } else if copper_territory.craft_by_hand_if_needed(self.tick.as_mut(RestrictMutToken(()))) {
            println!("handcrafting copper ore");
            // The tick has advanced
        } else if let Some(f) = self.mut_tick_queue.pop_front() {
            f(self, mut_token)
        } else {
            self.tick.as_mut(mut_token).advance();
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
                println!("{}", self.tick());
                return ret;
            }
            self.tick_fwd();
        }
    }
    pub fn complete_all(&mut self) {
        loop {
            if self.queue.is_all_done() {
                println!("{}", self.tick());
                return;
            }
            self.tick_fwd();
        }
    }
}
