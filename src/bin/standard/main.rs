#![forbid(unsafe_code)]
#![feature(
    generic_const_exprs,
    marker_trait_attr,
    never_type,
    specialization,
    try_blocks,
    try_trait_v2
)]
#![allow(incomplete_features)]
use indexmap::IndexMap;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    ops::ControlFlow,
};

pub use rustorio::{
    Bundle, HandRecipe, Recipe, ResearchPoint, Resource, ResourceType, Technology, Tick,
    buildings::{Assembler, Furnace, Lab},
    gamemodes::Standard,
    recipes::{
        AssemblerRecipe, CopperSmelting, CopperWireRecipe, ElectronicCircuitRecipe, FurnaceRecipe,
        IronSmelting, PointRecipe, RedScienceRecipe, SteelSmelting,
    },
    research::{PointsTechnology, RedScience, SteelTechnology},
    resources::{Copper, CopperOre, CopperWire, ElectronicCircuit, Iron, IronOre, Point, Steel},
    territory::{Miner, Territory},
};
pub use rustorio_engine::{recipe::RecipeEx, research::TechRecipe};

mod analysis;
mod crafting;
mod machine;
mod recipes;
mod resources;
mod runtime;
mod scheduler;
pub use analysis::*;
pub use crafting::*;
pub use machine::*;
pub use recipes::*;
pub use resources::*;
pub use runtime::*;
pub use scheduler::*;

type GameMode = Standard;

type StartingResources = <GameMode as rustorio::GameMode>::StartingResources;

pub struct Victory(<GameMode as rustorio::GameMode>::VictoryResources);

impl SingleMakeable for Victory {
    type Input = <GameMode as rustorio::GameMode>::VictoryResources;
    fn make_from_input(_state: &mut GameState, input: Self::Input) -> Self {
        Victory(input)
    }
}

fn main() {
    fn user_main(
        tick: Tick,
        starting_resources: StartingResources,
    ) -> (Tick, <GameMode as rustorio::GameMode>::VictoryResources) {
        let (tick, v) = GameState::new(tick, starting_resources).play();
        (tick, v.0)
    }
    rustorio::play::<GameMode>(user_main);
}

impl Resources {
    fn new(starting_resources: StartingResources) -> (Resources, Producers) {
        let StartingResources {
            iron,
            iron_territory,
            copper_territory,
            steel_technology,
        } = starting_resources;

        let mut resources = Resources::default();
        let mut producers = Producers::default();
        resources.resource().add(iron);
        resources.reusable().set(steel_technology);
        producers.add_territory(iron_territory);
        producers.add_territory(copper_territory);
        (resources, producers)
    }
}

/// A store of various producers.
#[derive(Default)]
pub struct Producers {
    producers: IndexMap<TypeId, Box<dyn ErasedProducer>>,
}

pub trait ErasedProducer: Any {
    fn name(&self) -> String;
    fn available_parallelism(&self) -> u32;
    fn load(&self) -> usize;
    fn report_load(&mut self, tick: &Tick) -> String;
    fn update(&mut self, tick: &Tick, waiters: &mut CallBackQueue);
    fn scale_up_if_needed(&mut self) -> Option<Box<dyn FnOnce(&mut GameState)>>;
}
impl<P: Producer> ErasedProducer for ProducerWithQueue<P> {
    fn name(&self) -> String {
        P::name()
    }
    fn available_parallelism(&self) -> u32 {
        self.producer.available_parallelism()
    }
    fn load(&self) -> usize {
        self.queue.len()
    }
    fn report_load(&mut self, tick: &Tick) -> String {
        let load = self.load();
        if let Some(s) = self.producer.report_load(tick) {
            format!("{load} -- {s}")
        } else {
            load.to_string()
        }
    }
    fn update(&mut self, tick: &Tick, waiters: &mut CallBackQueue) {
        self.update(tick, waiters);
    }
    fn scale_up_if_needed(&mut self) -> Option<Box<dyn FnOnce(&mut GameState)>> {
        self.scale_up_if_needed()
    }
}

pub trait ErasedHandProducer: Any {
    fn craft_by_hand_if_needed(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick>;
}
impl<P: HandProducer> ErasedHandProducer for ProducerWithQueue<P> {
    fn craft_by_hand_if_needed(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick> {
        self.craft_by_hand_if_needed(tick)
    }
}

impl Producers {
    fn or_insert_producer<P: Producer>(
        &mut self,
        f: impl FnOnce() -> P,
    ) -> &mut ProducerWithQueue<P> {
        let storage: &mut (dyn ErasedProducer + 'static) = self
            .producers
            .entry(TypeId::of::<P>())
            .or_insert_with(|| Box::new(ProducerWithQueue::new(f())))
            .as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    fn insert_producer<P: Producer>(&mut self, p: P) {
        self.producers
            .insert(TypeId::of::<P>(), Box::new(ProducerWithQueue::new(p)));
    }
    fn get_producer<P: Producer>(&mut self) -> &mut ProducerWithQueue<P> {
        let storage: &mut (dyn ErasedProducer + 'static) =
            self.producers.get_mut(&TypeId::of::<P>()).unwrap().as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    pub fn iter_producers(&mut self) -> impl Iterator<Item = &mut dyn ErasedProducer> {
        self.producers.values_mut().map(|s| s.as_mut())
    }

    pub fn machine<M: Machine + Makeable>(&mut self) -> &mut ProducerWithQueue<MultiMachine<M>> {
        self.or_insert_producer(|| MultiMachine::<M>::default())
    }
    pub fn add_territory<O: ResourceType + Any>(&mut self, t: Territory<O>) {
        self.insert_producer(t);
    }
    pub fn territory<O: ResourceType + Any>(&mut self) -> &mut ProducerWithQueue<Territory<O>> {
        self.get_producer()
    }
    pub fn hand_crafter<R: HandRecipe + ConstRecipe>(
        &mut self,
    ) -> &mut ProducerWithQueue<HandCrafter<R>> {
        self.or_insert_producer(|| HandCrafter::<R>::default())
    }
    pub fn once_maker<O: OnceMakeable + Any>(&mut self) -> &mut ProducerWithQueue<OnceMaker<O>> {
        self.or_insert_producer(|| OnceMaker::<O>::default())
    }

    pub fn with_hand_producers(
        &mut self,
        mut f: impl FnMut(&mut dyn ErasedHandProducer) -> ControlFlow<AdvancedTick>,
    ) -> ControlFlow<AdvancedTick> {
        f(self.territory::<IronOre>())?;
        f(self.territory::<CopperOre>())?;
        f(self.machine::<Assembler<CopperWireRecipe>>())?;
        f(self.hand_crafter::<RedScienceRecipe>())?;
        ControlFlow::Continue(())
    }
}

impl GameState {
    fn play(mut self) -> (Tick, Victory) {
        let p = Priority(4);
        // Start with this one otherwise we're stuck.
        self.scale_up::<MultiMachine<Furnace<IronSmelting>>>(p);
        // self.add_furnace::<IronSmelting>(p);
        // self.add_furnace::<CopperSmelting>(p);

        self.add_miner::<IronOre>(p);
        // self.add_miner::<CopperOre>(p);

        self.add_furnace::<IronSmelting>(p);
        self.add_furnace::<CopperSmelting>(p);

        let h = self.add_assembler::<CopperWireRecipe>(p);
        self.complete(h);

        // self.add_furnace::<IronSmelting>(p);
        // self.add_furnace::<IronSmelting>(p);
        self.add_miner::<IronOre>(p);
        self.add_miner::<IronOre>(p);

        // self.scale_up::<OnceMaker<SteelSmelting>>(p);
        // self.add_furnace::<IronSmelting>(p);
        // self.add_miner::<CopperOre>(p);
        // self.add_miner::<CopperOre>(p);
        // self.add_miner::<CopperOre>(p);
        // self.add_miner::<IronOre>(p);
        // self.add_furnace::<CopperSmelting>(p);

        // self.add_assembler::<ElectronicCircuitRecipe>(p);

        self.add_lab::<SteelTechnology>(p);
        // self.scale_up::<OnceMaker<PointRecipe>>(p);

        self.add_furnace::<SteelSmelting>(p);
        // self.add_furnace::<SteelSmelting>(p);

        let victory = self.make(Priority(0));

        let victory: Victory = self.complete(victory);

        self.graph.set_display_root::<Victory>();
        println!("{}", self.graph);

        (self.tick.into_inner(), victory)
    }
}
