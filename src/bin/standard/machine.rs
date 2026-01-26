use std::{
    any::{Any, type_name},
    marker::PhantomData,
    mem,
};

use rustorio::{
    Recipe, Resource, ResourceType, Technology, Tick,
    buildings::{Assembler, Furnace, Lab},
    recipes::{AssemblerRecipe, FurnaceRecipe},
    territory::{Miner, Territory},
};
use rustorio_engine::research::TechRecipe;

use crate::{
    GameState, Resources,
    crafting::{ConstRecipe, Makeable},
    scheduler::WakeHandle,
};

pub trait Machine {
    type Recipe: Recipe<Inputs: MachineInputs>;
    fn inputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Inputs;
    fn outputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Outputs;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load(&mut self, tick: &Tick) -> u32
    where
        Self::Recipe: ConstRecipe,
    {
        Self::Recipe::input_load(self.inputs(tick))
    }
    fn type_name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

impl<R: FurnaceRecipe + Recipe<Inputs: MachineInputs>> Machine for Furnace<R> {
    type Recipe = R;
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<R: AssemblerRecipe + Recipe<Inputs: MachineInputs>> Machine for Assembler<R> {
    type Recipe = R;
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<T: Technology> Machine for Lab<T>
where
    TechRecipe<T>: Recipe<Inputs: MachineInputs>,
{
    type Recipe = TechRecipe<T>;
    fn inputs(&mut self, tick: &Tick) -> &mut <TechRecipe<T> as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <TechRecipe<T> as Recipe>::Outputs {
        self.outputs(tick)
    }
}

pub trait MachineInputs {
    fn input_count(&self) -> u32;
}
impl<R1: ResourceType> MachineInputs for (Resource<R1>,) {
    fn input_count(&self) -> u32 {
        self.0.amount()
    }
}
impl<R1: ResourceType, R2: ResourceType> MachineInputs for (Resource<R1>, Resource<R2>) {
    fn input_count(&self) -> u32 {
        // Doesn't matter which we pick since we're comparing along the same resource.
        self.0.amount()
    }
}

/// A crafting slot in a machine of type `M`.
#[derive(Debug)]
pub struct MachineSlot<M>(usize, PhantomData<M>);

impl<M> Copy for MachineSlot<M> {}
impl<M> Clone for MachineSlot<M> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Default)]
pub enum MachineStorage<M> {
    /// The machine isn't there; craft by hand.
    #[default]
    NoMachine,
    /// The machine is being constructed; just wait for it to be ready.
    InConstruction,
    /// The machine is there.
    Present(Vec<M>),
    /// We removed that machine; error when trying to craft.
    Removed,
}

impl<M> MachineStorage<M> {
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present(vec) if !vec.is_empty())
    }
    pub fn needs_construction(&self) -> bool {
        match self {
            MachineStorage::NoMachine => true,
            _ => false,
        }
    }
    pub fn add(&mut self, m: M) {
        eprintln!("adding a {}", std::any::type_name::<M>());
        if !self.is_present() {
            *self = Self::Present(vec![])
        }
        let Self::Present(vec) = self else {
            unreachable!()
        };
        vec.push(m);
    }
    // fn iter(&mut self) -> impl Iterator<Item = &mut M> {
    //     match self {
    //         Self::Present(vec) => Some(vec),
    //         _ => None,
    //     }
    //     .into_iter()
    //     .flatten()
    // }
    // fn max_load(&mut self, tick: &Tick) -> Option<(u32, &str)>
    // where
    //     M: Machine,
    //     M::Recipe: ConstRecipe,
    // {
    //     self.iter()
    //         .map(|m| (m.input_load(tick), m.type_name()))
    //         .max()
    // }

    pub fn request(&mut self, tick: &Tick) -> Option<MachineSlot<M>>
    where
        M: Machine,
    {
        match self {
            Self::NoMachine | Self::InConstruction => None,
            // Find the least loaded machine
            Self::Present(vec) => {
                let res = vec
                    .iter_mut()
                    .map(|m| m.inputs(tick).input_count())
                    .enumerate()
                    .min_by_key(|(_, input_count)| *input_count)
                    .map(|(id, _)| MachineSlot(id, PhantomData));
                // if vec.len() > 1 {
                //     let (min, max) = vec
                //         .iter_mut()
                //         .map(|m| m.inputs(tick).input_count())
                //         .minmax_by_key(|input_count| *input_count)
                //         .into_option()
                //         .unwrap();
                //     eprintln!(
                //         "picked {:?} among {} {} ([{min}, {max}])",
                //         res,
                //         vec.len(),
                //         std::any::type_name::<M>()
                //     );
                // }
                res
            }
            Self::Removed => panic!("trying to craft with a removed {}", type_name::<M>()),
        }
    }
    pub fn get(&mut self, id: MachineSlot<M>) -> &mut M {
        match self {
            Self::Present(vec) => &mut vec[id.0],
            _ => panic!(),
        }
    }
    pub fn take_map<N>(&mut self, f: impl Fn(M) -> N) -> MachineStorage<N> {
        match mem::replace(self, Self::Removed) {
            Self::NoMachine | Self::InConstruction => MachineStorage::NoMachine,
            Self::Present(vec) => MachineStorage::Present(vec.into_iter().map(f).collect()),
            Self::Removed => MachineStorage::Removed,
        }
    }
}

impl GameState {
    pub fn add_miner<R: ResourceType + Any>(
        &mut self,
        f: fn(&mut Resources) -> &mut Option<Territory<R>>,
    ) -> WakeHandle<()> {
        let inputs = self.make();
        self.map(inputs, move |state, (iron, copper)| {
            let miner = Miner::build(iron, copper);
            f(&mut state.resources)
                .as_mut()
                .unwrap()
                .add_miner(&state.tick, miner)
                .unwrap();
        })
    }

    pub fn add_machine<M: Machine + Makeable>(&mut self) -> WakeHandle<()> {
        let machine_store = self.resources.machine_store.for_type::<M>();
        if machine_store.needs_construction() {
            // Avoid double-creating the first machine.
            *machine_store = MachineStorage::InConstruction;
        }
        let machine = self.make();
        self.map(machine, move |state, machine: M| {
            state.resources.machine_store.for_type().add(machine);
        })
    }
    pub fn add_assembler<R>(&mut self) -> WakeHandle<()>
    where
        R: AssemblerRecipe,
        Assembler<R>: Machine + Makeable,
    {
        self.add_machine::<Assembler<R>>()
    }
    pub fn add_furnace<R>(&mut self) -> WakeHandle<()>
    where
        R: FurnaceRecipe,
        Furnace<R>: Machine + Makeable,
    {
        self.add_machine::<Furnace<R>>()
    }
    pub fn add_lab<T>(&mut self) -> WakeHandle<()>
    where
        T: Technology,
        Lab<T>: Machine + Makeable,
    {
        self.add_machine::<Lab<T>>()
    }
}
