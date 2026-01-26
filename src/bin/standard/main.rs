#![forbid(unsafe_code)]
#![feature(generic_const_exprs, try_trait_v2, never_type)]
#![allow(incomplete_features)]
use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

use rustorio::{
    self, Bundle, Resource, ResourceType, Tick,
    gamemodes::Standard,
    recipes::{
        CopperSmelting, CopperWireRecipe, ElectronicCircuitRecipe, IronSmelting, PointRecipe,
        SteelSmelting,
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

/// A store of various machines.
#[derive(Default)]
pub struct MachineStore {
    /// Maps the type id of `M` to a `Box<MachineStorage<M>>`.
    map: HashMap<TypeId, Box<dyn StoredMachine>>,
}
impl MachineStore {
    pub fn for_type<M: Machine + Any>(&mut self) -> &mut MachineStorage<M> {
        let storage: &mut (dyn StoredMachine + 'static) = self
            .map
            .entry(TypeId::of::<M>())
            .or_insert_with(|| Box::new(MachineStorage::<M>::default()))
            .as_mut();
        let storage: &mut (dyn Any + 'static) = storage;
        storage.downcast_mut().unwrap()
    }
    pub fn get<M: Machine + Any>(&mut self, id: MachineSlot<M>) -> &mut M {
        self.for_type::<M>().get(id)
    }
    pub fn iter(&mut self) -> impl Iterator<Item = &mut dyn StoredMachine> {
        self.map.values_mut().map(|s| s.as_mut())
    }
}

pub trait StoredMachine: Any {}
impl<M: Machine + Any> StoredMachine for MachineStorage<M> {}

#[derive(Default)]
struct Resources {
    iron_territory: Option<Territory<IronOre>>,
    copper_territory: Option<Territory<CopperOre>>,

    steel_technology: Option<SteelTechnology>,
    points_technology: Option<PointsTechnology>,
    steel_smelting: Option<SteelSmelting>,
    points_recipe: Option<PointRecipe>,

    resource_store: ResourceStore,
    machine_store: MachineStore,
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
        resources.steel_technology = Some(steel_technology);
        resources.iron_territory = Some(iron_territory);
        resources.copper_territory = Some(copper_territory);
        resources
    }
}

impl GameState {
    fn play(mut self) -> (Tick, Bundle<Point, 200>) {
        let h = self.add_furnace::<IronSmelting>();
        self.complete(h);

        let h = self.add_furnace::<CopperSmelting>();
        self.complete(h);

        self.add_miner(|r| &mut r.iron_territory);
        self.add_miner(|r| &mut r.copper_territory);

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

        let _points: WakeHandle<Bundle<Point, 10>> = self.make();
        self.complete_all();
        todo!("WIP: {}", self.tick.cur())
        // let points = self.complete(points);
        // (self.tick, points)
    }
}
