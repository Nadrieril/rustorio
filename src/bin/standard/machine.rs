use std::{
    any::{Any, type_name},
    cell::RefCell,
    collections::VecDeque,
    marker::PhantomData,
    mem,
    rc::Rc,
    sync::Arc,
};

use rustorio::{
    Recipe, ResourceType, Technology, Tick,
    buildings::{Assembler, Furnace, Lab},
    recipes::{AssemblerRecipe, FurnaceRecipe},
    resources::IronOre,
    territory::{Miner, Territory},
};
use rustorio_engine::research::TechRecipe;

use crate::{
    GameState, Resources,
    crafting::{ConstRecipe, Makeable},
    scheduler::{WakeHandle, WakeHandleId},
};

pub trait Machine {
    type Recipe: ConstRecipe;
    fn inputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Inputs;
    fn outputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Outputs;
    fn add_bundled_inputs(
        &mut self,
        tick: &Tick,
        inputs: <Self::Recipe as ConstRecipe>::BundledInputs,
    ) {
        Self::Recipe::add_inputs(self.inputs(tick), inputs);
    }
    fn get_bundled_outputs(
        &mut self,
        tick: &Tick,
    ) -> Option<<Self::Recipe as ConstRecipe>::BundledOutputs> {
        Self::Recipe::get_outputs(&mut self.outputs(tick))
    }
}

impl<R: FurnaceRecipe + Recipe + ConstRecipe> Machine for Furnace<R> {
    type Recipe = R;
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<R: AssemblerRecipe + Recipe + ConstRecipe> Machine for Assembler<R> {
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
    TechRecipe<T>: Recipe + ConstRecipe,
{
    type Recipe = TechRecipe<T>;
    fn inputs(&mut self, tick: &Tick) -> &mut <TechRecipe<T> as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <TechRecipe<T> as Recipe>::Outputs {
        self.outputs(tick)
    }
}

// /// `Miners` don't quite work like machines but it would be super nice to integrate them in our
// /// load balancing setup, so for each miner we add a corresponding miner-machine that serves as
// /// interface. Making a new one of these will add a miner to the right territory.
// struct IronOreMachine(Rc<RefCell<Territory<IronOre>>>);

// struct IronMiningRecipe;
// impl Recipe for IronMiningRecipe {}

// impl Machine for IronOreMachine {
//     type Recipe = IronMiningRecipe;

//     fn inputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Inputs {
//         todo!()
//     }

//     fn outputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Outputs {}
// }

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
    Present(Vec<MachineWithQueue<M>>),
    /// We removed that machine; error when trying to craft.
    Removed,
}

pub struct MachineWithQueue<M> {
    pub machine: M,
    /// Queue of items waiting for this machine to produce an output. Only the first item in the
    /// queue polls the machine; the rest are each waiting on the next one to save on useless
    /// polling.
    pub queue: VecDeque<WakeHandleId>,
}

impl<M: Machine> MachineWithQueue<M> {
    fn new(machine: M) -> Self {
        Self {
            machine,
            queue: Default::default(),
        }
    }
    pub fn add_inputs(&mut self, tick: &Tick, inputs: <M::Recipe as ConstRecipe>::BundledInputs) {
        self.machine.add_bundled_inputs(tick, inputs);
    }
    pub fn get_outputs(
        &mut self,
        tick: &Tick,
    ) -> Option<<M::Recipe as ConstRecipe>::BundledOutputs> {
        self.machine.get_bundled_outputs(tick)
    }
}

impl<M: Machine> MachineStorage<M> {
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
        vec.push(MachineWithQueue::new(m));
    }
    fn iter(&self) -> impl Iterator<Item = &MachineWithQueue<M>> {
        match self {
            Self::Present(vec) => Some(vec),
            _ => None,
        }
        .into_iter()
        .flatten()
    }
    #[expect(unused)]
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut MachineWithQueue<M>> {
        match self {
            Self::Present(vec) => Some(vec),
            _ => None,
        }
        .into_iter()
        .flatten()
    }
    /// Compute the total number of clients waiting for output.
    pub fn total_load(&self) -> usize {
        self.iter().map(|m| m.queue.len()).sum()
    }

    pub fn request(&mut self, _tick: &Tick) -> Option<MachineSlot<M>> {
        match self {
            Self::NoMachine | Self::InConstruction => None,
            // Find the least loaded machine
            Self::Present(vec) => vec
                .iter_mut()
                .map(|m| m.queue.len())
                .enumerate()
                .min_by_key(|(_, queue_len)| *queue_len)
                .map(|(id, _)| MachineSlot(id, PhantomData)),
            Self::Removed => panic!("trying to craft with a removed {}", type_name::<M>()),
        }
    }
    pub fn get(&mut self, id: MachineSlot<M>) -> &mut MachineWithQueue<M> {
        match self {
            Self::Present(vec) => &mut vec[id.0],
            _ => panic!(),
        }
    }
    pub fn take_map<N: Machine>(&mut self, f: impl Fn(M) -> N) -> MachineStorage<N> {
        match mem::replace(self, Self::Removed) {
            Self::NoMachine | Self::InConstruction => MachineStorage::NoMachine,
            Self::Present(vec) => MachineStorage::Present(
                vec.into_iter()
                    .map(|m| f(m.machine))
                    .map(MachineWithQueue::new)
                    .collect(),
            ),
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
