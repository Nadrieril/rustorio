use std::{any::Any, cmp::Reverse, collections::VecDeque, mem, ops::ControlFlow};

use itertools::Itertools;

use crate::*;

pub fn type_name<T: Any>() -> String {
    let str = std::any::type_name::<T>();
    str.split('<')
        .map(|str| {
            str.split('<')
                .map(|str| str.split("::").last().unwrap())
                .format("<")
        })
        .format("<")
        .to_string()
}

pub trait Machine: Any {
    type Recipe: ConstRecipe;
    fn inputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Inputs;
    fn outputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Outputs;

    /// The number of input bundles currently in the machine.
    fn input_load(&mut self, tick: &Tick) -> u32 {
        <Self::Recipe as ConstRecipe>::BundledInputs::bundle_count(self.inputs(tick))
    }
    fn add_inputs(&mut self, tick: &Tick, inputs: <Self::Recipe as ConstRecipe>::BundledInputs) {
        <Self::Recipe as ConstRecipe>::BundledInputs::add(self.inputs(tick), inputs);
    }
    /// Used to load-balance across machines of the same type.
    fn pop_inputs(&mut self, tick: &Tick) -> Option<<Self::Recipe as ConstRecipe>::BundledInputs> {
        <Self::Recipe as ConstRecipe>::BundledInputs::bundle(&mut self.inputs(tick))
    }
    /// Used when we handcrafted some values, to have somewhere to store them.
    fn add_outputs(&mut self, tick: &Tick, outputs: <Self::Recipe as ConstRecipe>::BundledOutputs) {
        <Self::Recipe as ConstRecipe>::BundledOutputs::add(self.outputs(tick), outputs);
    }
    fn pop_outputs(
        &mut self,
        tick: &Tick,
    ) -> Option<<Self::Recipe as ConstRecipe>::BundledOutputs> {
        <Self::Recipe as ConstRecipe>::BundledOutputs::bundle(&mut self.outputs(tick))
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

pub enum MultiMachine<M: Machine> {
    /// We have no machines; we may craft by hand if the recipe allows.
    NoMachine {
        /// Inputs gathered while there was no constructed machine.
        inputs: Vec<<M::Recipe as ConstRecipe>::BundledInputs>,
        /// Outputs handcrafted while there was no constructed machine (if relevant).
        outputs: Vec<<M::Recipe as ConstRecipe>::BundledOutputs>,
    },
    /// We have machines.
    Present(Vec<M>),
    /// We removed those machines; error when trying to craft.
    Removed,
}

impl<M: Machine> MultiMachine<M> {
    pub fn is_present(&self) -> bool {
        matches!(self, Self::Present(vec) if !vec.is_empty())
    }
    pub fn count(&self) -> u32 {
        match self {
            MultiMachine::NoMachine { .. } | MultiMachine::Removed => 0,
            MultiMachine::Present(machines) => machines.len() as u32,
        }
    }

    pub fn add(&mut self, tick: &Tick, mut m: M) {
        println!("adding a {}", type_name::<M>());
        match self {
            MultiMachine::NoMachine { inputs, outputs } => {
                for input in mem::take(inputs) {
                    m.add_inputs(tick, input);
                }
                for output in mem::take(outputs) {
                    m.add_outputs(tick, output);
                }
                *self = Self::Present(vec![m])
            }
            MultiMachine::Present(items) => {
                // Get inputs from the other machines to average out the load.
                let total_load: u32 = items.iter_mut().map(|m| m.input_load(tick)).sum();
                let average_load: u32 = total_load / ((items.len() + 1) as u32);
                for n in items.iter_mut() {
                    while n.input_load(tick) > average_load
                        && let Some(input) = n.pop_inputs(tick)
                    {
                        m.add_inputs(tick, input);
                    }
                }
                items.push(m)
            }
            MultiMachine::Removed => {
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
    pub fn take_map<N: Machine>(&mut self, f: impl Fn(M) -> N) -> MultiMachine<N> {
        match mem::replace(self, Self::Removed) {
            Self::NoMachine { .. } => MultiMachine::default(),
            Self::Present(vec) => MultiMachine::Present(vec.into_iter().map(|m| f(m)).collect()),
            Self::Removed => MultiMachine::Removed,
        }
    }

    fn poll(&mut self, tick: &Tick) -> Option<<M::Recipe as ConstRecipe>::BundledOutputs> {
        match self {
            MultiMachine::NoMachine { outputs, .. } => outputs.pop(),
            MultiMachine::Present(machines) => {
                for m in machines {
                    if let Some(o) = m.pop_outputs(tick) {
                        return Some(o);
                    }
                }
                None
            }
            MultiMachine::Removed => None,
        }
    }
}

impl<M: Machine> Default for MultiMachine<M> {
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
    type Input: Any;
    type Output: Any;
    fn name() -> String;

    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self>;

    /// Count the number of producing entities (miners, assemblers, ..) available.
    fn available_parallelism(&self) -> u32;
    /// Count the number of production cycles it takes to produce an extra producing entity for
    /// this. Used as a heuristic for scaling up.
    fn self_cost(&self) -> u32;

    /// Detailed load reporting, if available.
    fn report_load(&mut self, _tick: &Tick) -> Option<String> {
        None
    }

    fn add_inputs(&mut self, tick: &Tick, inputs: Self::Input);

    /// Update the producer and yield an output if one is ready.
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output>;

    /// Turn a receiver of outputs into a receiver of inputs.
    fn feed(p: Priority, sink: Sink<Self::Output>) -> StateSink<Self::Input> {
        StateSink::from_fn(move |state, inputs| {
            Self::get_ref(&mut state.resources).feed(
                &state.tick,
                &mut state.queue,
                p,
                inputs,
                sink,
            );
        })
    }

    /// Schedule the addition of a new producing entity of this type. This is called when the load
    /// becomes too high compared to the available parallelism.
    fn scale_up(&self, _p: Priority) -> Box<dyn FnOnce(&mut GameState) -> WakeHandle<()>> {
        Box::new(|state| state.never())
    }
    /// Trigger a scaling up. This ensures we don't scale up many times in parallel.
    fn trigger_scale_up(p: Priority) -> Box<dyn FnOnce(&mut GameState)> {
        Box::new(move |state| {
            eprintln!("scaling up {}", type_name::<Self>());
            let this = Self::get_ref(&mut state.resources);
            this.scaling_up += 1;
            let h = this.producer.scale_up(p)(state);
            state.map(h, |state, _| {
                Self::get_ref(&mut state.resources).scaling_up -= 1;
            });
        })
    }
}

impl<R: HandRecipe + ConstRecipe + Any> Producer for HandCrafter<R> {
    type Input = <R as ConstRecipe>::BundledInputs;
    type Output = <R as ConstRecipe>::BundledOutputs;
    fn name() -> String {
        type_name::<R>()
    }
    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.hand_crafter()
    }
    fn available_parallelism(&self) -> u32 {
        1
    }
    fn self_cost(&self) -> u32 {
        0
    }

    fn add_inputs(&mut self, _tick: &Tick, inputs: Self::Input) {
        self.inputs.push(inputs);
    }
    fn poll(&mut self, _tick: &Tick) -> Option<Self::Output> {
        self.outputs.pop()
    }
}

impl<Ore: ResourceType + Any> Producer for Territory<Ore>
where
    Miner: CostIn<Ore>,
{
    type Input = ();
    type Output = (Bundle<Ore, 1>,);
    fn name() -> String {
        type_name::<Ore>()
    }
    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.territory::<Ore>()
    }
    fn available_parallelism(&self) -> u32 {
        self.num_miners()
    }
    fn self_cost(&self) -> u32 {
        <Miner as CostIn<Ore>>::COST
    }

    fn add_inputs(&mut self, _tick: &Tick, _inputs: Self::Input) {}
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output> {
        self.resources(tick).bundle().ok().map(|x| (x,))
    }

    fn scale_up(&self, p: Priority) -> Box<dyn FnOnce(&mut GameState) -> WakeHandle<()>> {
        let num_miners = self.num_miners();
        let max_miners = self.max_miners();
        Box::new(move |state| {
            let this = Self::get_ref(&mut state.resources);
            if num_miners + this.scaling_up <= max_miners {
                let miner = state.make(p);
                state.map(miner, move |state, miner| {
                    Self::get_ref(&mut state.resources)
                        .producer
                        .add_miner(&state.tick, miner)
                        .unwrap();
                })
            } else {
                state.never()
            }
        })
    }
}

impl<M> Producer for MultiMachine<M>
where
    M: Machine + Makeable,
{
    type Input = <M::Recipe as ConstRecipe>::BundledInputs;
    type Output = <M::Recipe as ConstRecipe>::BundledOutputs;
    fn name() -> String {
        type_name::<M::Recipe>()
    }
    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.machine::<M>()
    }
    fn available_parallelism(&self) -> u32 {
        self.count()
    }
    fn self_cost(&self) -> u32 {
        todo!()
    }
    fn report_load(&mut self, tick: &Tick) -> Option<String> {
        match self {
            MultiMachine::Present(machines) => Some(
                machines
                    .iter_mut()
                    .map(|m| m.input_load(tick).to_string())
                    .format(" ")
                    .to_string(),
            ),
            MultiMachine::NoMachine { .. } => None,
            MultiMachine::Removed => None,
        }
    }

    fn add_inputs(&mut self, tick: &Tick, inputs: Self::Input) {
        self.add_inputs(tick, inputs);
    }
    fn poll(&mut self, tick: &Tick) -> Option<Self::Output> {
        self.poll(tick)
    }

    fn scale_up(&self, p: Priority) -> Box<dyn FnOnce(&mut GameState) -> WakeHandle<()>> {
        Box::new(move |state| {
            let machine = state.make(p);
            state.map(machine, move |state, machine: M| {
                state.resources.machine().producer.add(&state.tick, machine);
            })
        })
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
impl<Ore: ResourceType + Any> HandProducer for Territory<Ore>
where
    Miner: CostIn<Ore>,
{
    fn can_craft_automatically(&self) -> bool {
        self.num_miners() > 0
    }
    fn craft_by_hand(&mut self, tick: &mut Tick) -> ControlFlow<AdvancedTick> {
        let out = self.hand_mine::<1>(tick);
        self.resources(tick).add(out);
        ControlFlow::Break(AdvancedTick)
    }
}
impl<M> HandProducer for MultiMachine<M>
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

/// Wrapper for resources that we only need to make once. The first time we query the resource will
/// trigger the making of this, and subsequent times will reuse the already-produced resource.
pub struct TheFirstTime<R>(pub R);

/// Producer that represents items that are produced only once.
pub struct OnceMaker<O> {
    is_started: bool,
    output: Option<O>,
}
impl<O> OnceMaker<O> {
    pub fn ensure_made(state: &mut GameState, p: Priority) -> WakeHandle<()>
    where
        O: Clone,
        TheFirstTime<O>: Makeable,
    {
        let this = &mut Self::get_ref(&mut state.resources).producer;
        if !this.is_started {
            this.is_started = true;
            let first_time = state.make(p);
            state.map(first_time, |state, TheFirstTime(o)| {
                Self::get_ref(&mut state.resources).producer.output = Some(o);
            })
        } else {
            state.nowait(())
        }
    }
}
impl<O> Default for OnceMaker<O> {
    fn default() -> Self {
        Self {
            is_started: false,
            output: None,
        }
    }
}

impl<O: Clone + Any> Producer for OnceMaker<O>
where
    TheFirstTime<O>: Makeable,
{
    type Input = ();
    type Output = O;
    fn name() -> String {
        type_name::<Self>()
    }
    fn get_ref(resources: &mut Resources) -> &mut ProducerWithQueue<Self> {
        resources.once_maker()
    }
    fn available_parallelism(&self) -> u32 {
        self.output.is_some() as u32
    }
    fn self_cost(&self) -> u32 {
        0
    }

    fn add_inputs(&mut self, _tick: &Tick, _inputs: Self::Input) {}
    fn poll(&mut self, _tick: &Tick) -> Option<Self::Output> {
        self.output.clone()
    }

    fn scale_up(&self, p: Priority) -> Box<dyn FnOnce(&mut GameState) -> WakeHandle<()>> {
        Box::new(move |state| {
            let this = &mut Self::get_ref(&mut state.resources).producer;
            if !this.is_started {
                this.is_started = true;
                let first_time = state.make(p);
                state.map(first_time, |state, TheFirstTime(o)| {
                    Self::get_ref(&mut state.resources).producer.output = Some(o);
                })
            } else {
                state.nowait(())
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(pub u16);

/// A producer along with a queue of items waiting on it.
pub struct ProducerWithQueue<P: Producer> {
    pub producer: P,
    /// Keep sorted by priority.
    pub queue: VecDeque<(Sink<P::Output>, Priority)>,
    /// Number of producing entities we're in the process of building.
    pub scaling_up: u32,
}

impl<P: Producer> ProducerWithQueue<P> {
    pub fn new(producer: P) -> Self {
        Self {
            producer,
            queue: Default::default(),
            scaling_up: Default::default(),
        }
    }

    fn enqueue(
        &mut self,
        tick: &Tick,
        waiters: &mut CallBackQueue,
        sink: Sink<P::Output>,
        p: Priority,
    ) {
        if self.queue.is_empty()
            && let Some(output) = self.producer.poll(tick)
        {
            sink.give(waiters, output);
        } else {
            self.queue.push_back((sink, p));
            self.queue
                .make_contiguous()
                .sort_by_key(|(_, p)| Reverse(*p));
        }
    }
    /// Feed the producer some inputs and somewhere to put the generated output.
    pub fn feed(
        &mut self,
        tick: &Tick,
        waiters: &mut CallBackQueue,
        p: Priority,
        inputs: P::Input,
        sink: Sink<P::Output>,
    ) {
        self.producer.add_inputs(tick, inputs);
        self.enqueue(tick, waiters, sink, p);
    }

    pub fn update(&mut self, tick: &Tick, waiters: &mut CallBackQueue) {
        while !self.queue.is_empty()
            && let Some(output) = self.producer.poll(tick)
        {
            let (sink, _) = self.queue.pop_front().unwrap();
            sink.give(waiters, output);
        }
    }

    /// Checks if scaling up may be needed. If so, return a function to be called on the game state
    /// to schedule a scale up.
    pub fn scale_up_if_needed(&mut self) -> Option<Box<dyn FnOnce(&mut GameState)>> {
        if self.queue.len()
            > (self.producer.available_parallelism() + self.scaling_up * 12) as usize * 4
        {
            let p = self.queue.front().unwrap().1;
            let p = Priority(p.0 + 1);
            Some(P::trigger_scale_up(p))
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
            println!("crafting by hand with {}", P::name());
            self.producer.craft_by_hand(tick)
        } else {
            ControlFlow::Continue(())
        }
    }
}

impl GameState {
    /// Enqueue the creation of a new producing entity for this producer type.
    pub fn scale_up<P: Producer>(&mut self, p: Priority) {
        <P>::trigger_scale_up(p)(self)
    }

    pub fn add_miner<Ore: ResourceType + Any>(&mut self, p: Priority)
    where
        Miner: CostIn<Ore>,
    {
        self.scale_up::<Territory<Ore>>(p)
    }

    pub fn add_machine<M: Machine + Makeable>(&mut self, p: Priority) -> WakeHandle<()> {
        // self.scale_up::<MultiMachine<M>>(p)
        // If we use `trigger_scale_up` then we lose some parallelism :(
        <MultiMachine<M>>::scale_up(&self.resources.machine().producer, p)(&mut *self)
    }
    pub fn add_assembler<R>(&mut self, p: Priority) -> WakeHandle<()>
    where
        R: AssemblerRecipe,
        Assembler<R>: Machine + Makeable,
    {
        self.add_machine::<Assembler<R>>(p)
    }
    pub fn add_furnace<R>(&mut self, p: Priority) -> WakeHandle<()>
    where
        R: FurnaceRecipe,
        Furnace<R>: Machine + Makeable,
    {
        self.add_machine::<Furnace<R>>(p)
    }
    pub fn add_lab<T>(&mut self, p: Priority) -> WakeHandle<()>
    where
        T: Technology,
        Lab<T>: Machine + Makeable,
    {
        self.add_machine::<Lab<T>>(p)
    }
}
