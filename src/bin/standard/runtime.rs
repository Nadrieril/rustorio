use std::{any::Any, collections::VecDeque, ops::Deref};

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
        let mut_token = RestrictMutToken(());
        if let Some(f) = self.mut_tick_queue.pop_front() {
            f(self, mut_token)
        } else {
            self.tick.as_mut(mut_token).advance();
        }
        self.check_waiters();
        self.report_loads();
    }
    pub fn report_loads(&mut self) {
        if true {
            return;
        }
        // let r = &mut self.resources;
        // macro_rules! max_load {
        //     ($($m:ident,)*) => {
        //         [$(
        //             r.$m.max_load(&self.tick),
        //         )*]
        //     };
        // }
        // // let loads = max_load!(
        // //     iron_furnace,
        // //     copper_furnace,
        // //     steel_furnace,
        // //     copper_wire_assembler,
        // //     elec_circuit_assembler,
        // //     points_assembler,
        // //     steel_lab,
        // //     points_lab,
        // // );
        // let (max_load, name) = loads.iter().flatten().max().unwrap();
        // eprintln!("{}: a {name} has load {max_load}", self.tick.as_ref());
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
