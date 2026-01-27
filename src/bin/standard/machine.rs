use std::{
    any::{Any, type_name},
    collections::VecDeque,
    marker::PhantomData,
    mem,
};

use rustorio::{
    Bundle, Recipe, ResourceType, Technology, Tick,
    buildings::{Assembler, Furnace, Lab},
    recipes::{AssemblerRecipe, FurnaceRecipe},
    territory::{Miner, Territory},
};
use rustorio_engine::research::TechRecipe;

use crate::{
    GameState, Resources,
    crafting::{ConstRecipe, Makeable},
    scheduler::{WaiterQueue, WakeHandle},
};

pub trait Machine: Any {
    type Recipe: ConstRecipe;
    fn inputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Inputs;
    fn outputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Outputs;

    /// The number of input bundles currently in the machine.
    fn input_load(&mut self, tick: &Tick) -> u32 {
        Self::Recipe::input_load(self.inputs(tick))
    }
    fn add_inputs(&mut self, tick: &Tick, inputs: <Self::Recipe as ConstRecipe>::BundledInputs) {
        Self::Recipe::add_inputs(self.inputs(tick), inputs);
    }
    fn get_outputs(
        &mut self,
        tick: &Tick,
    ) -> Option<<Self::Recipe as ConstRecipe>::BundledOutputs> {
        Self::Recipe::get_outputs(&mut self.outputs(tick))
    }
}

impl<R: FurnaceRecipe + ConstRecipe + Any> Machine for Furnace<R> {
    type Recipe = R;
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<R: AssemblerRecipe + ConstRecipe + Any> Machine for Assembler<R> {
    type Recipe = R;
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<T: Technology + Any> Machine for Lab<T>
where
    TechRecipe<T>: ConstRecipe,
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
    Present(Vec<M>),
    /// We removed that machine; error when trying to craft.
    Removed,
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
        vec.push(m);
    }
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut M> {
        match self {
            Self::Present(vec) => Some(vec),
            _ => None,
        }
        .into_iter()
        .flatten()
    }

    pub fn request(&mut self, tick: &Tick) -> Option<MachineSlot<M>> {
        match self {
            Self::NoMachine | Self::InConstruction => None,
            // Find the least loaded machine
            Self::Present(vec) => vec
                .iter_mut()
                .map(|m| m.input_load(tick))
                .enumerate()
                .min_by_key(|(_, queue_len)| *queue_len)
                .map(|(id, _)| MachineSlot(id, PhantomData)),
            Self::Removed => panic!("trying to craft with a removed {}", type_name::<M>()),
        }
    }
    pub fn get(&mut self, id: MachineSlot<M>) -> &mut M {
        match self {
            Self::Present(vec) => &mut vec[id.0],
            _ => panic!(),
        }
    }
    pub fn take_map<N: Machine>(&mut self, f: impl Fn(M) -> N) -> MachineStorage<N> {
        match mem::replace(self, Self::Removed) {
            Self::NoMachine | Self::InConstruction => MachineStorage::NoMachine,
            Self::Present(vec) => MachineStorage::Present(vec.into_iter().map(|m| f(m)).collect()),
            Self::Removed => MachineStorage::Removed,
        }
    }
}

/// An entity that produces outputs.
pub trait Producer: Any {
    type Output: Any;
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output>;
}

impl<Ore: ResourceType + Any> Producer for Territory<Ore> {
    type Output = Bundle<Ore, 1>;
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output> {
        self.resources(tick).bundle().ok()
    }
}

impl<M> Producer for MachineStorage<M>
where
    M: Machine,
{
    type Output = <M::Recipe as ConstRecipe>::BundledOutputs;
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output> {
        for m in self.iter_mut() {
            if let Some(o) = m.get_outputs(tick) {
                return Some(o);
            }
        }
        None
    }
}

/// A producer that can be run by hand if needed.
pub trait HandProducer: Producer {
    /// Whether the producer has the right machines to produce output automatically.
    fn can_craft_automatically(&self) -> bool;
    /// Run the producer by hand once.
    fn craft_by_hand(&mut self, tick: &mut Tick);
}

impl<Ore: ResourceType + Any> HandProducer for Territory<Ore> {
    fn can_craft_automatically(&self) -> bool {
        self.num_miners() > 0
    }
    fn craft_by_hand(&mut self, tick: &mut Tick) {
        let out = self.hand_mine::<1>(tick);
        self.resources(tick).add(out);
    }
}
// impl<M: Machine> HandProducer for MachineStorage<M> {
//     fn can_craft_automatically(&self) -> bool {
//         self.is_present()
//     }
//     fn craft_by_hand(&mut self, tick: &mut Tick) {
//         // TODO: problem is getting inputs and storing outputs
//         todo!()
//         // let out = self.hand_mine::<1>(tick);
//         // self.resources(tick).add(out);

//         //     let out = M::Recipe::craft(tick, inputs);
//         //     self.resources.resource_store.get().add(out);
//     }
// }

/// A producer along with a queue of items waiting on it.
pub struct ProducerWithQueue<P: Producer> {
    pub producer: P,
    pub queue: VecDeque<WakeHandle<P::Output>>,
}

impl<P: Producer> ProducerWithQueue<P> {
    pub fn new(producer: P) -> Self {
        Self {
            producer,
            queue: Default::default(),
        }
    }

    pub fn enqueue(&mut self, tick: &Tick, waiters: &mut WaiterQueue, h: WakeHandle<P::Output>) {
        if self.queue.is_empty()
            && let Some(output) = self.producer.poll(tick)
        {
            waiters.set_output(h, output);
        } else {
            self.queue.push_back(h);
        }
    }

    pub fn update(&mut self, tick: &Tick, waiters: &mut WaiterQueue) {
        while !self.queue.is_empty()
            && let Some(output) = self.producer.poll(tick)
        {
            let h = self.queue.pop_front().unwrap();
            waiters.set_output(h, output);
        }
    }

    /// If the producer has a non-empty queue and can't produce output automatically, craft by hand
    /// instead.
    pub fn craft_by_hand_if_needed(&mut self, tick: &mut Tick) -> bool
    where
        P: HandProducer,
    {
        if !self.producer.can_craft_automatically() && !self.queue.is_empty() {
            self.producer.craft_by_hand(tick);
            true
        } else {
            false
        }
    }
}

impl GameState {
    pub fn add_miner<R: ResourceType + Any>(
        &mut self,
        f: fn(&mut Resources) -> Option<&mut Territory<R>>,
    ) -> WakeHandle<()> {
        let inputs = self.make();
        self.map(inputs, move |state, (iron, copper)| {
            let miner = Miner::build(iron, copper);
            f(&mut state.resources)
                .unwrap()
                .add_miner(&state.tick, miner)
                .unwrap();
        })
    }

    pub fn add_machine<M: Machine + Makeable>(&mut self) -> WakeHandle<()> {
        let machine_store = &mut self.resources.machine_store.for_type::<M>().producer;
        if machine_store.needs_construction() {
            // Avoid double-creating the first machine.
            *machine_store = MachineStorage::InConstruction;
        }
        let machine = self.make();
        self.map(machine, move |state, machine: M| {
            state
                .resources
                .machine_store
                .for_type()
                .producer
                .add(machine);
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
