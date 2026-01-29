#![forbid(unsafe_code)]
#![feature(generic_const_exprs, try_trait_v2, never_type)]
#![allow(incomplete_features)]
use std::{
    any::{Any, TypeId, type_name_of_val},
    collections::HashMap,
    ops::ControlFlow,
};

use crafting::{ConstRecipe, Makeable};
use indexmap::IndexMap;
use rustorio::{
    self, Bundle, HandRecipe, Resource, ResourceType, Tick,
    buildings::Assembler,
    gamemodes::Standard,
    recipes::{
        CopperSmelting, CopperWireRecipe, ElectronicCircuitRecipe, IronSmelting, PointRecipe,
        RedScienceRecipe, SteelSmelting,
    },
    research::{PointsTechnology, SteelTechnology},
    resources::{CopperOre, IronOre, Point},
    territory::Territory,
};

mod scheduler;
use scheduler::*;
mod crafting;
mod machine;
use machine::*;
mod runtime;
use runtime::*;

type GameMode = Standard;

type StartingResources = <GameMode as rustorio::GameMode>::StartingResources;

fn main() {
    fn user_main(tick: Tick, starting_resources: StartingResources) -> (Tick, Bundle<Point, 200>) {
        GameState::new(tick, starting_resources).play()
    }
    rustorio::play::<GameMode>(user_main);
}

/// A store of various resources.
#[derive(Default)]
pub struct ResourceStore {
    /// Maps the type id of `R` to a `Box<Resource<R>>`.
    map: HashMap<TypeId, Box<dyn Any>>,
}
impl ResourceStore {
    pub fn get<R: ResourceType + Any>(&mut self) -> &mut Resource<R> {
        self.map
            .entry(TypeId::of::<R>())
            .or_insert_with(|| Box::new(Resource::<R>::new_empty()))
            .downcast_mut()
            .unwrap()
    }
}

/// A store of producers.
#[derive(Default)]
pub struct ProducerStore {
    map: IndexMap<TypeId, Box<dyn ErasedProducer>>,
}
impl ProducerStore {
    fn or_insert<P: Producer>(&mut self, f: impl FnOnce() -> P) -> &mut ProducerWithQueue<P> {
        let storage: &mut (dyn ErasedProducer + 'static) = self
            .map
            .entry(TypeId::of::<P>())
            .or_insert_with(|| Box::new(ProducerWithQueue::new(f())))
            .as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    fn insert<P: Producer>(&mut self, p: P) {
        self.map
            .insert(TypeId::of::<P>(), Box::new(ProducerWithQueue::new(p)));
    }
    fn get<P: Producer>(&mut self) -> &mut ProducerWithQueue<P> {
        let storage: &mut (dyn ErasedProducer + 'static) =
            self.map.get_mut(&TypeId::of::<P>()).unwrap().as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    pub fn iter(&mut self) -> impl Iterator<Item = &mut dyn ErasedProducer> {
        self.map.values_mut().map(|s| s.as_mut())
    }

    pub fn machine<M: Machine + Makeable>(&mut self) -> &mut ProducerWithQueue<MachineStorage<M>> {
        self.or_insert(|| MachineStorage::<M>::default())
    }
    pub fn add_territory<O: ResourceType + Any>(&mut self, t: Territory<O>) {
        self.insert(t);
    }
    pub fn territory<O: ResourceType + Any>(&mut self) -> &mut ProducerWithQueue<Territory<O>> {
        self.get()
    }
    pub fn hand_crafter<R: HandRecipe + ConstRecipe>(
        &mut self,
    ) -> &mut ProducerWithQueue<HandCrafter<R>> {
        self.or_insert(|| HandCrafter::<R>::default())
    }
    pub fn once_maker<O: Clone + Any>(&mut self) -> &mut ProducerWithQueue<OnceMaker<O>> {
        self.or_insert(|| OnceMaker::<O>::default())
    }
}

pub trait ErasedProducer: Any {
    fn name(&self) -> &'static str;
    fn available_parallelism(&self) -> u32;
    fn load(&self) -> usize;
    fn report_load(&mut self, tick: &Tick) -> String;
    fn update(&mut self, tick: &Tick, waiters: &mut WaiterQueue);
    fn scale_up_if_needed(&mut self) -> Option<Box<dyn FnOnce(&mut GameState)>>;
}
impl<P: Producer> ErasedProducer for ProducerWithQueue<P> {
    fn name(&self) -> &'static str {
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
    fn update(&mut self, tick: &Tick, waiters: &mut WaiterQueue) {
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

#[derive(Default)]
pub struct Resources {
    steel_technology: Option<SteelTechnology>,
    points_technology: Option<PointsTechnology>,

    resource_store: ResourceStore,
    producers: ProducerStore,
}

impl Resources {
    fn new(starting_resources: StartingResources) -> Self {
        let StartingResources {
            iron,
            iron_territory,
            copper_territory,
            steel_technology,
        } = starting_resources;

        let mut resources = Resources::default();
        resources.resource_store.get().add(iron);
        resources.producers.add_territory(iron_territory);
        resources.producers.add_territory(copper_territory);
        resources.steel_technology = Some(steel_technology);
        resources
    }

    pub fn producers(&mut self) -> impl Iterator<Item = &mut dyn ErasedProducer> {
        self.producers.iter()
    }

    pub fn with_hand_producers(
        &mut self,
        mut f: impl FnMut(&mut dyn ErasedHandProducer) -> ControlFlow<AdvancedTick>,
    ) -> ControlFlow<AdvancedTick> {
        f(self.producers.territory::<IronOre>())?;
        f(self.producers.territory::<CopperOre>())?;
        f(self.producers.machine::<Assembler<CopperWireRecipe>>())?;
        f(self.producers.hand_crafter::<RedScienceRecipe>())?;
        ControlFlow::Continue(())
    }
}

impl GameState {
    fn play(mut self) -> (Tick, Bundle<Point, 200>) {
        let p = Priority(1);
        // Start with this one otherwise we're stuck.
        // <MachineStorage<Furnace<IronSmelting>>>::trigger_scale_up(&mut self);
        self.add_furnace::<IronSmelting>(p);
        self.add_furnace::<CopperSmelting>(p);

        self.add_miner::<IronOre>(p);
        self.add_miner::<CopperOre>(p);

        self.add_furnace::<IronSmelting>(p);
        self.add_furnace::<CopperSmelting>(p);

        let h = self.add_assembler::<CopperWireRecipe>(p);
        self.complete(h);

        self.add_furnace::<IronSmelting>(p);
        self.add_furnace::<IronSmelting>(p);
        self.add_miner::<IronOre>(p);
        self.add_miner::<IronOre>(p);

        self.add_furnace::<IronSmelting>(p);
        // self.add_miner::<CopperOre>(p);
        // self.add_miner::<CopperOre>(p);
        // self.add_miner::<CopperOre>(p);
        // self.add_miner::<IronOre>(p);
        // self.add_furnace::<CopperSmelting>(p);

        self.add_assembler::<ElectronicCircuitRecipe>(p);

        self.add_lab::<SteelTechnology>(p);

        self.add_furnace::<SteelSmelting>(p);
        // self.add_furnace::<SteelSmelting>(p);

        self.add_assembler::<PointRecipe>(p);

        let points = self.make(Priority(0));

        eprintln!("starting!");
        let points: Bundle<Point, 100> = self.complete(points);

        panic!(
            "WIP: in {} ticks, got {}",
            self.tick.cur(),
            type_name_of_val(&points),
        );
        // (self.tick, points)
    }
}
