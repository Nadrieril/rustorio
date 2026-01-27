use std::{
    any::{Any, type_name},
    collections::VecDeque,
    mem,
    ops::ControlFlow,
};

use rustorio::{
    Bundle, HandRecipe, Recipe, ResourceType, Technology, Tick,
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
    /// Used when we handcrafted some values, to have somewhere to store them.
    fn add_outputs(&mut self, tick: &Tick, outputs: <Self::Recipe as ConstRecipe>::BundledOutputs) {
        Self::Recipe::add_outputs(self.outputs(tick), outputs);
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

pub enum MachineStorage<M: Machine> {
    /// The machine isn't there; craft by hand.
    NoMachine {
        /// Inputs gathered while there was no constructed machine.
        inputs: Vec<<M::Recipe as ConstRecipe>::BundledInputs>,
        /// Outputs handcrafted while there was no constructed machine (if relevant).
        outputs: Vec<<M::Recipe as ConstRecipe>::BundledOutputs>,
    },
    /// The machine is there.
    Present(Vec<M>),
    /// We removed that machine; error when trying to craft.
    Removed,
}

impl<M: Machine> MachineStorage<M> {
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present(vec) if !vec.is_empty())
    }
    pub fn count(&self) -> u32 {
        match self {
            MachineStorage::NoMachine { .. } | MachineStorage::Removed => 0,
            MachineStorage::Present(machines) => machines.len() as u32,
        }
    }

    pub fn add(&mut self, tick: &Tick, mut m: M) {
        println!("adding a {}", std::any::type_name::<M>());
        match self {
            MachineStorage::NoMachine { inputs, outputs } => {
                for input in mem::take(inputs) {
                    m.add_inputs(tick, input);
                }
                for output in mem::take(outputs) {
                    m.add_outputs(tick, output);
                }
                *self = Self::Present(vec![m])
            }
            MachineStorage::Present(items) => items.push(m),
            MachineStorage::Removed => {
                panic!("trying to craft with a removed {}", type_name::<M>())
            }
        }
    }

    pub fn add_inputs(&mut self, tick: &Tick, input: <M::Recipe as ConstRecipe>::BundledInputs) {
        match self {
            Self::NoMachine { inputs, .. } => inputs.push(input),
            // Find the least loaded machine
            Self::Present(vec) => {
                vec.iter_mut()
                    .map(|m| (m.input_load(tick), m))
                    .min_by_key(|&(load, _)| load)
                    .map(|(_, m)| m)
                    .unwrap()
                    .add_inputs(tick, input);
            }
            Self::Removed => panic!("trying to craft with a removed {}", type_name::<M>()),
        }
    }
    pub fn take_map<N: Machine>(&mut self, f: impl Fn(M) -> N) -> MachineStorage<N> {
        match mem::replace(self, Self::Removed) {
            Self::NoMachine { .. } => MachineStorage::default(),
            Self::Present(vec) => MachineStorage::Present(vec.into_iter().map(|m| f(m)).collect()),
            Self::Removed => MachineStorage::Removed,
        }
    }

    fn poll(&mut self, tick: &Tick) -> Option<<M::Recipe as ConstRecipe>::BundledOutputs> {
        match self {
            MachineStorage::NoMachine { outputs, .. } => outputs.pop(),
            MachineStorage::Present(machines) => {
                for m in machines {
                    if let Some(o) = m.get_outputs(tick) {
                        return Some(o);
                    }
                }
                None
            }
            MachineStorage::Removed => None,
        }
    }
}

impl<M: Machine> Default for MachineStorage<M> {
    fn default() -> Self {
        Self::NoMachine {
            inputs: Default::default(),
            outputs: Default::default(),
        }
    }
}

/// Producer that represents handcrafting.
pub struct HandCrafter<R: ConstRecipe> {
    pub inputs: Vec<<R as ConstRecipe>::BundledInputs>,
    outputs: Vec<<R as ConstRecipe>::BundledOutputs>,
}

impl<R: ConstRecipe> Default for HandCrafter<R> {
    fn default() -> Self {
        Self {
            inputs: Default::default(),
            outputs: Default::default(),
        }
    }
}

/// An entity that produces outputs.
pub trait Producer: Any + Sized {
    type Output: Any;
    fn name() -> &'static str;

    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self>;

    /// Count the number of producing entities (miners, assemblers, ..) available.
    fn available_parallelism(&self) -> u32;

    /// Update the producer and yield an output if one is ready.
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output>;

    /// Schedule the addition of a new producing entity of this type. This is called when the load
    /// becomes too high compared to the available parallelism.
    fn scale_up(state: &mut GameState) -> WakeHandle<()>;
}

impl<R: HandRecipe + ConstRecipe + Any> Producer for HandCrafter<R> {
    type Output = <R as ConstRecipe>::BundledOutputs;

    fn name() -> &'static str {
        std::any::type_name::<R>()
    }

    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.producers.hand_crafter()
    }

    fn available_parallelism(&self) -> u32 {
        0
    }

    fn poll(&mut self, _tick: &Tick) -> Option<Self::Output> {
        self.outputs.pop()
    }

    fn scale_up(state: &mut GameState) -> WakeHandle<()> {
        state.never()
    }
}

impl<Ore: ResourceType + Any> Producer for Territory<Ore> {
    type Output = (Bundle<Ore, 1>,);
    fn name() -> &'static str {
        std::any::type_name::<Ore>()
    }

    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.producers.territory::<Ore>()
    }

    fn available_parallelism(&self) -> u32 {
        self.num_miners()
    }

    fn poll(&mut self, tick: &Tick) -> Option<Self::Output> {
        self.resources(tick).bundle().ok().map(|x| (x,))
    }

    fn scale_up(state: &mut GameState) -> WakeHandle<()> {
        state.add_miner::<Ore>()
    }
}

impl<M> Producer for MachineStorage<M>
where
    M: Machine + Makeable,
{
    type Output = <M::Recipe as ConstRecipe>::BundledOutputs;
    fn name() -> &'static str {
        std::any::type_name::<M::Recipe>()
    }

    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.producers.machine::<M>()
    }

    fn available_parallelism(&self) -> u32 {
        self.count()
    }

    fn poll(&mut self, tick: &Tick) -> Option<Self::Output> {
        self.poll(tick)
    }

    fn scale_up(state: &mut GameState) -> WakeHandle<()> {
        state.add_machine::<M>()
    }
}

/// Token indicating that an operation caused the tick to advance.
pub struct AdvancedTick;

/// A producer that can be run by hand if needed.
pub trait HandProducer: Producer {
    /// Whether the producer has the right machines to produce output automatically.
    fn can_craft_automatically(&self) -> bool;
    /// Run the producer by hand once. Returns whether we advanced the tick.
    fn craft_by_hand(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick>;
}

impl<R> HandProducer for HandCrafter<R>
where
    R: HandRecipe + ConstRecipe,
    R: HandRecipe<InputBundle = <R as ConstRecipe>::BundledInputs>,
    R: HandRecipe<OutputBundle = <R as ConstRecipe>::BundledOutputs>,
{
    fn can_craft_automatically(&self) -> bool {
        false
    }
    fn craft_by_hand(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick> {
        if let Some(inputs) = self.inputs.pop() {
            let out = R::craft(tick, inputs);
            self.outputs.push(out);
            ControlFlow::Break(AdvancedTick)
        } else {
            ControlFlow::Continue(())
        }
    }
}
impl<Ore: ResourceType + Any> HandProducer for Territory<Ore> {
    fn can_craft_automatically(&self) -> bool {
        self.num_miners() > 0
    }
    fn craft_by_hand(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick> {
        let out = self.hand_mine::<1>(tick);
        self.resources(tick).add(out);
        ControlFlow::Break(AdvancedTick)
    }
}
impl<M> HandProducer for MachineStorage<M>
where
    M: Machine + Makeable,
    M::Recipe: HandRecipe<InputBundle = <M::Recipe as ConstRecipe>::BundledInputs>,
    M::Recipe: HandRecipe<OutputBundle = <M::Recipe as ConstRecipe>::BundledOutputs>,
{
    fn can_craft_automatically(&self) -> bool {
        self.is_present()
    }
    fn craft_by_hand(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick> {
        let Self::NoMachine { inputs, outputs } = self else {
            panic!("can't craft by hand?")
        };
        let Some(input) = inputs.pop() else {
            return ControlFlow::Continue(());
        };
        let out = M::Recipe::craft(tick, input);
        outputs.push(out);
        ControlFlow::Break(AdvancedTick)
    }
}

/// A producer along with a queue of items waiting on it.
pub struct ProducerWithQueue<P: Producer> {
    pub producer: P,
    pub queue: VecDeque<WakeHandle<P::Output>>,
    /// Whether we're in the process of adding a new producing entity (so we don't add several at
    /// the same time).
    pub is_scaling_up: bool,
}

impl<P: Producer> ProducerWithQueue<P> {
    pub fn new(producer: P) -> Self {
        Self {
            producer,
            queue: Default::default(),
            is_scaling_up: Default::default(),
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

    /// Checks if scaling up may be needed. If so, return a function to be called on the game state
    /// to schedule a scale up.
    pub fn scale_up_if_needed(&mut self) -> Option<fn(&mut GameState)> {
        if !self.is_scaling_up
            && false
            && self.queue.len() > self.producer.available_parallelism() as usize * 5
        {
            fn scale_up<P: Producer>(state: &mut GameState) {
                if !P::get_ref(&mut state.resources).is_scaling_up {
                    P::get_ref(&mut state.resources).is_scaling_up = true;
                    let h = P::scale_up(state);
                    state.map(h, |state, _| {
                        P::get_ref(&mut state.resources).is_scaling_up = false;
                    });
                }
            }
            Some(scale_up::<P>)
        } else {
            None
        }
    }

    /// If the producer has a non-empty queue and can't produce output automatically, craft by hand
    /// instead. Return whether we advanced the time.
    pub fn craft_by_hand_if_needed(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick>
    where
        P: HandProducer,
    {
        if !self.producer.can_craft_automatically() && !self.queue.is_empty() {
            self.producer.craft_by_hand(tick)
        } else {
            ControlFlow::Continue(())
        }
    }
}

impl GameState {
    pub fn add_miner<R: ResourceType + Any>(&mut self) -> WakeHandle<()> {
        let inputs = self.make();
        self.map(inputs, move |state, (iron, copper)| {
            let miner = Miner::build(iron, copper);
            state
                .resources
                .producers
                .territory::<R>()
                .producer
                .add_miner(&state.tick, miner)
                .unwrap();
        })
    }

    pub fn add_machine<M: Machine + Makeable>(&mut self) -> WakeHandle<()> {
        let machine = self.make();
        self.map(machine, move |state, machine: M| {
            state
                .resources
                .producers
                .machine()
                .producer
                .add(&state.tick, machine);
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
