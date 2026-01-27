#![forbid(unsafe_code)]
#![feature(generic_const_exprs, try_trait_v2, never_type)]
#![allow(incomplete_features)]
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    ops::ControlFlow,
};

use crafting::{ConstRecipe, Makeable};
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
    /// Maps the type id of `M` to a `MachineStorage<M>`, of `O` to `Territory<O>`, of `R` to
    /// `HandCrafter<R>`.
    map: HashMap<TypeId, Box<dyn ErasedProducer>>,
}
impl ProducerStore {
    pub fn machine<M: Machine + Makeable>(&mut self) -> &mut ProducerWithQueue<MachineStorage<M>> {
        let storage: &mut (dyn ErasedProducer + 'static) = self
            .map
            .entry(TypeId::of::<M>())
            .or_insert_with(|| Box::new(ProducerWithQueue::new(MachineStorage::<M>::default())))
            .as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    pub fn add_territory<O: ResourceType + Any>(&mut self, t: Territory<O>) {
        self.map
            .insert(TypeId::of::<O>(), Box::new(ProducerWithQueue::new(t)));
    }
    pub fn territory<O: ResourceType + Any>(&mut self) -> &mut ProducerWithQueue<Territory<O>> {
        let storage: &mut (dyn ErasedProducer + 'static) =
            self.map.get_mut(&TypeId::of::<O>()).unwrap().as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    pub fn hand_crafter<R: HandRecipe + ConstRecipe>(
        &mut self,
    ) -> &mut ProducerWithQueue<HandCrafter<R>> {
        let storage: &mut (dyn ErasedProducer + 'static) = self
            .map
            .entry(TypeId::of::<R>())
            .or_insert_with(|| Box::new(ProducerWithQueue::new(HandCrafter::<R>::default())))
            .as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    pub fn iter(&mut self) -> impl Iterator<Item = &mut dyn ErasedProducer> {
        self.map.values_mut().map(|s| s.as_mut())
    }
}

pub trait ErasedProducer: Any {
    fn name(&self) -> &'static str;
    fn available_parallelism(&self) -> u32;
    fn load(&self) -> usize;
    fn update(&mut self, tick: &Tick, waiters: &mut WaiterQueue);
    fn scale_up_if_needed(&mut self) -> Option<fn(&mut GameState)>;
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
    fn update(&mut self, tick: &Tick, waiters: &mut WaiterQueue) {
        self.update(tick, waiters);
    }
    fn scale_up_if_needed(&mut self) -> Option<fn(&mut GameState)> {
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
    steel_smelting: Option<SteelSmelting>,
    points_recipe: Option<PointRecipe>,

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
        let h = self.add_furnace::<IronSmelting>();
        self.complete(h);

        let h = self.add_furnace::<CopperSmelting>();
        self.complete(h);

        self.add_miner::<IronOre>();
        self.add_miner::<CopperOre>();

        self.add_furnace::<IronSmelting>();
        self.add_furnace::<IronSmelting>();
        self.add_furnace::<CopperSmelting>();
        let h = self.add_assembler::<CopperWireRecipe>();
        self.complete(h);

        self.add_assembler::<ElectronicCircuitRecipe>();

        self.add_lab::<SteelTechnology>();

        // self.add_miner(|r| &mut r.iron_territory);
        // self.add_furnace(IronSmelting);
        // self.add_miner(|r| &mut r.copper_territory);
        // self.add_furnace(CopperSmelting);

        // self.add_assembler(CopperWireRecipe);
        // self.add_assembler(ElectronicCircuitRecipe);

        self.add_furnace::<SteelSmelting>();

        // self.add_lab(|r| &r.points_technology, |r| &mut r.points_lab);

        self.add_assembler::<PointRecipe>();

        // self.add_miner(|r| &mut r.iron_territory);
        // self.add_furnace(IronSmelting);

        let points: WakeHandle<Bundle<Point, 10>> = self.make();
        let _ = self.complete(points);
        todo!("WIP: {}", self.tick.cur())
        // let points = self.complete(points);
        // (self.tick, points)
    }
}
