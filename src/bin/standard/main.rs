#![forbid(unsafe_code)]
#![feature(generic_const_exprs, try_trait_v2, never_type)]
#![allow(incomplete_features)]
use std::{
    any::{Any, TypeId},
    collections::{HashMap, VecDeque},
    mem,
    ops::Deref,
};

use itertools::Itertools;
use rustorio::{
    self, Bundle, HandRecipe, Recipe, ResearchPoint, Resource, ResourceType, Technology, Tick,
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
use rustorio_engine::research::TechRecipe;

mod scheduler;
use scheduler::*;

type GameMode = Standard;

type StartingResources = <GameMode as rustorio::GameMode>::StartingResources;

fn main() {
    fn user_main(tick: Tick, starting_resources: StartingResources) -> (Tick, Bundle<Point, 200>) {
        GameState::new(tick, starting_resources).play()
    }
    rustorio::play::<GameMode>(user_main);
}

// Const fns because direct field access is not allowed in const exprs.
const fn tup1_field0<A: Copy>(x: (A,)) -> A {
    x.0
}
const fn tup2_field0<A: Copy, B: Copy>(x: (A, B)) -> A {
    x.0
}
const fn tup2_field1<A: Copy, B: Copy>(x: (A, B)) -> B {
    x.1
}

/// Trait to compute statically-counted inputs and outputs. The const generic is needed because the
/// impls would otherwise be considered to overlap.
trait ConstRecipe<const INPUT_N: u32>: Recipe {
    type BundledInputs;
    type BundledOutputs;
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs);
    fn get_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs>;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load(input: &Self::Inputs) -> u32;
}

impl<R, I, O> ConstRecipe<1> for R
where
    I: ResourceType,
    O: ResourceType,
    R: Recipe<Inputs = (Resource<I>,), InputAmountsType = (u32,)>,
    R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
    [(); { tup1_field0(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup1_field0(R::OUTPUT_AMOUNTS) } as usize]:,
{
    type BundledInputs = (Bundle<I, { tup1_field0(R::INPUT_AMOUNTS) }>,);
    type BundledOutputs = (Bundle<O, { tup1_field0(R::OUTPUT_AMOUNTS) }>,);
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs) {
        to.0.add(i.0);
    }
    fn get_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs> {
        Some((from.0.bundle().ok()?,))
    }
    fn input_load(input: &Self::Inputs) -> u32 {
        input.0.amount() / R::INPUT_AMOUNTS.0
    }
}

impl<R, I1, I2, O> ConstRecipe<2> for R
where
    I1: ResourceType,
    I2: ResourceType,
    O: ResourceType,
    R: Recipe<Inputs = (Resource<I1>, Resource<I2>), InputAmountsType = (u32, u32)>,
    R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
    [(); { tup2_field0(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup2_field1(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup1_field0(R::OUTPUT_AMOUNTS) } as usize]:,
{
    type BundledInputs = (
        Bundle<I1, { tup2_field0(R::INPUT_AMOUNTS) }>,
        Bundle<I2, { tup2_field1(R::INPUT_AMOUNTS) }>,
    );
    type BundledOutputs = (Bundle<O, { tup1_field0(R::OUTPUT_AMOUNTS) }>,);
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs) {
        to.0.add(i.0);
        to.1.add(i.1);
    }
    fn get_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs> {
        Some((from.0.bundle().ok()?,))
    }
    fn input_load(input: &Self::Inputs) -> u32 {
        input.0.amount() / R::INPUT_AMOUNTS.0
    }
}

trait Machine {
    type Recipe: Recipe<Inputs: MachineInputs>;
    fn inputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Inputs;
    fn outputs(&mut self, tick: &Tick) -> &mut <Self::Recipe as Recipe>::Outputs;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load<const N: u32>(&mut self, tick: &Tick) -> u32
    where
        Self::Recipe: ConstRecipe<N>,
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

trait MachineInputs {
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

/// Wrapper to restrict mutable access.
struct RestrictMut<T>(T);

/// Must only be created when a tick is explicitly requested. Jobs which require mutable access to
/// the `Tick` should enqueue themselves to `GameState.mut_tick_queue`.
struct RestrictMutToken(());

impl<T> RestrictMut<T> {
    fn new(x: T) -> Self {
        Self(x)
    }
    fn as_ref(&self) -> &T {
        &self.0
    }
    fn as_mut(&mut self, _: RestrictMutToken) -> &mut T {
        &mut self.0
    }
}
impl<T> Deref for RestrictMut<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

struct GameState {
    tick: RestrictMut<Tick>,
    /// Advancing time during the waiter updates risks skipping updates. So instead we require that
    /// jobs which need mutable ownership of the `Tick` be put in a separate queue.
    mut_tick_queue: VecDeque<Box<dyn FnOnce(&mut GameState, RestrictMutToken)>>,
    resources: Resources,
    queue: WaiterQueue,
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

#[derive(Default)]
struct Resources {
    iron_territory: Option<Territory<IronOre>>,
    copper_territory: Option<Territory<CopperOre>>,

    steel_technology: Option<SteelTechnology>,
    points_technology: Option<PointsTechnology>,
    steel_smelting: Option<SteelSmelting>,
    points_recipe: Option<PointRecipe>,

    resource_store: ResourceStore,

    iron_furnace: MachineStorage<Furnace<IronSmelting>>,
    copper_furnace: MachineStorage<Furnace<CopperSmelting>>,
    steel_furnace: MachineStorage<Furnace<SteelSmelting>>,
    copper_wire_assembler: MachineStorage<Assembler<CopperWireRecipe>>,
    elec_circuit_assembler: MachineStorage<Assembler<ElectronicCircuitRecipe>>,
    points_assembler: MachineStorage<Assembler<PointRecipe>>,
    steel_lab: MachineStorage<Lab<SteelTechnology>>,
    points_lab: MachineStorage<Lab<PointsTechnology>>,
}

#[derive(Debug, Clone, Copy)]
struct MachineId(usize);

#[derive(Default)]
enum MachineStorage<M> {
    /// The machine isn't there; craft by hand.
    #[default]
    NoMachine,
    /// The machine is there.
    Present(Vec<M>),
    /// We removed that machine; error when trying to craft.
    Removed,
}

impl<M> MachineStorage<M> {
    fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }
    fn add(&mut self, m: M) {
        eprintln!("adding a {}", std::any::type_name::<M>());
        if !self.is_present() {
            *self = Self::Present(vec![])
        }
        let Self::Present(vec) = self else {
            unreachable!()
        };
        vec.push(m);
    }
    fn iter(&mut self) -> impl Iterator<Item = &mut M> {
        match self {
            Self::Present(vec) => Some(vec),
            _ => None,
        }
        .into_iter()
        .flatten()
    }

    fn request(&mut self, tick: &Tick) -> Option<MachineId>
    where
        M: Machine,
    {
        match self {
            Self::NoMachine => None,
            // Find the least loaded machine
            Self::Present(vec) => {
                let res = vec
                    .iter_mut()
                    .map(|m| m.inputs(tick).input_count())
                    .enumerate()
                    .min_by_key(|(_, input_count)| *input_count)
                    .map(|(id, _)| MachineId(id));
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
            Self::Removed => panic!("trying to craft with a removed machine"),
        }
    }
    fn get(&mut self, id: MachineId) -> &mut M {
        match self {
            Self::Present(vec) => &mut vec[id.0],
            _ => panic!(),
        }
    }
    fn take_map<N>(&mut self, f: impl Fn(M) -> N) -> MachineStorage<N> {
        match mem::replace(self, Self::Removed) {
            Self::NoMachine => MachineStorage::NoMachine,
            Self::Present(vec) => MachineStorage::Present(vec.into_iter().map(f).collect()),
            Self::Removed => MachineStorage::Removed,
        }
    }
}

impl GameState {
    fn new(mut tick: Tick, starting_resources: StartingResources) -> Self {
        let StartingResources {
            iron,
            iron_territory,
            copper_territory,
            steel_technology,
        } = starting_resources;

        tick.log(false);
        let mut resources = Resources::default();
        resources.resource_store.get().add(iron);
        resources.steel_technology = Some(steel_technology);
        resources.iron_territory = Some(iron_territory);
        resources.copper_territory = Some(copper_territory);
        GameState {
            tick: RestrictMut::new(tick),
            mut_tick_queue: Default::default(),
            queue: Default::default(),
            resources,
        }
    }

    fn tick(&self) -> &Tick {
        &self.tick
    }
    /// Enqueue an operation that requires advancing the tick time. It will be executed inside
    /// `tick_fwd` instead of plainly advancing the tick.
    fn with_mut_tick(&mut self, f: impl FnOnce(&mut GameState, RestrictMutToken) + Any) {
        self.mut_tick_queue.push_back(Box::new(f))
    }
    fn tick_fwd(&mut self) {
        let mut_token = RestrictMutToken(());
        if let Some(f) = self.mut_tick_queue.pop_front() {
            f(self, mut_token)
        } else {
            self.tick.as_mut(mut_token).advance();
        }
        self.check_waiters();
        self.report_loads();
    }
    fn report_loads(&mut self) {
        if true {
            return;
        }
        let r = &mut self.resources;
        macro_rules! max_load {
            ($($m:ident,)*) => {
                [$(
                    r.$m
                        .iter()
                        .map(|m| (m.input_load(&self.tick), m.type_name()))
                        .max(),
                )*]
            };
        }
        let loads = max_load!(
            iron_furnace,
            copper_furnace,
            steel_furnace,
            copper_wire_assembler,
            elec_circuit_assembler,
            points_assembler,
            steel_lab,
            points_lab,
        );
        let (max_load, name) = loads.iter().flatten().max().unwrap();
        eprintln!("{}: a {name} has load {max_load}", self.tick.as_ref());
    }
    #[expect(unused)]
    fn advance_by(&mut self, t: u64) {
        for _ in 0..t {
            self.tick_fwd()
        }
        println!("{}", self.tick());
    }
    fn complete<R: Any>(&mut self, h: WakeHandle<R>) -> R {
        loop {
            if let Some(ret) = self.queue.get(h) {
                println!("{}", self.tick());
                return ret;
            }
            self.tick_fwd();
        }
    }
    fn complete_all(&mut self) {
        loop {
            if self.queue.is_all_done() {
                println!("{}", self.tick());
                return;
            }
            self.tick_fwd();
        }
    }
}

impl GameState {
    pub fn collect_sum<const COUNT: u32, R: ResourceType + Any>(
        &mut self,
        handles: Vec<WakeHandle<Bundle<R, COUNT>>>,
    ) -> WakeHandle<Resource<R>> {
        let h = self.collect(handles);
        self.map(h, |_state, bundles| {
            bundles.into_iter().map(|b| b.to_resource()).sum()
        })
    }

    /// Waits until the function returns `true`.
    fn wait_until(&mut self, f: impl Fn(&mut GameState) -> bool + 'static) -> WakeHandle<()> {
        self.wait_for(move |state| f(state).then_some(()))
    }

    /// Waits until the given resource has the required amount then returns that amount of
    /// resource.
    fn wait_for_resource<const AMOUNT: u32, R: ResourceType + Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> &mut Resource<R> + 'static,
    ) -> WakeHandle<Bundle<R, AMOUNT>> {
        self.wait_for(move |state| f(state).bundle().ok())
    }

    /// Given a producer of a single bundle of an item, make a producer of a larger bundle.
    fn multiple<const SINGLE: u32, const COUNT: u32, R: ResourceType + Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> WakeHandle<Bundle<R, SINGLE>>,
    ) -> WakeHandle<Bundle<R, COUNT>> {
        assert_eq!(COUNT.rem_euclid(SINGLE), 0);
        let singles = (0..COUNT / SINGLE).map(|_| f(self)).collect_vec();
        let sum = self.collect_sum(singles);
        self.map(sum, |_, mut sum| sum.bundle().unwrap())
    }

    /// Craft an item using the provided machine.
    fn craft<const N: u32, R, M, O>(
        &mut self,
        inputs: R::BundledInputs,
        f: fn(&mut Resources) -> &mut MachineStorage<M>,
    ) -> WakeHandle<O>
    where
        R: ConstRecipe<N, BundledOutputs = (O,)> + Any,
        M: Machine<Recipe = R> + Any,
        O: Any,
    {
        let machine_id = self.wait_for(move |s| f(&mut s.resources).request(&s.tick));
        self.then(machine_id, move |state, machine_id| {
            let machine = f(&mut state.resources).get(machine_id);
            let machine_inputs = machine.inputs(&state.tick);
            R::add_inputs(machine_inputs, inputs);
            let out = state.wait_for(move |state| {
                let machine = f(&mut state.resources).get(machine_id);
                R::get_outputs(&mut machine.outputs(&state.tick))
            });
            state.map(out, |_, out| out.0)
        })
    }

    /// Craft an item using the provided machine. Tiny helper to avoid pesky 1-tuples.
    fn craft1<R, M, I, O>(
        &mut self,
        input: I,
        f: fn(&mut Resources) -> &mut MachineStorage<M>,
    ) -> WakeHandle<O>
    where
        R: ConstRecipe<1, BundledInputs = (I,), BundledOutputs = (O,)> + Any,
        M: Machine<Recipe = R> + Any,
        O: Any,
    {
        self.craft((input,), f)
    }

    fn add_machine<M: Any>(
        &mut self,
        machine_storage: fn(&mut Resources) -> &mut MachineStorage<M>,
        make_machine: impl FnOnce(&mut GameState) -> WakeHandle<M>,
    ) -> WakeHandle<()> {
        let machine = make_machine(self);
        self.map(machine, move |state, machine| {
            machine_storage(&mut state.resources).add(machine);
        })
    }
}

impl GameState {
    fn hand_mine<Ore: ResourceType + Any>(
        &mut self,
        territory: fn(&mut Resources) -> &mut Territory<Ore>,
    ) -> WakeHandle<Bundle<Ore, 1>> {
        self.with_mut_tick(move |state, mut_token| {
            let out: Bundle<Ore, 1> =
                territory(&mut state.resources).hand_mine(state.tick.as_mut(mut_token));
            state.resources.resource_store.get().add(out);
        });
        self.wait_for_resource(|state| state.resources.resource_store.get())
    }
    fn hand_craft<
        const AMOUNT: u32,
        R: HandRecipe<OutputBundle = (Bundle<O, AMOUNT>,)> + Any,
        O: ResourceType + Any,
    >(
        &mut self,
        inputs: R::InputBundle,
    ) -> WakeHandle<Bundle<O, AMOUNT>> {
        self.with_mut_tick(|state, mut_token| {
            let out = R::craft(state.tick.as_mut(mut_token), inputs).0;
            state.resources.resource_store.get().add(out);
        });
        self.wait_for_resource(|state| state.resources.resource_store.get())
    }

    fn iron_ore(&mut self) -> WakeHandle<Bundle<IronOre, 1>> {
        if self.resources.iron_territory.as_ref().unwrap().num_miners() == 0 {
            self.hand_mine(|r| r.iron_territory.as_mut().unwrap())
        } else {
            self.wait_for_resource(|state| {
                state
                    .resources
                    .iron_territory
                    .as_mut()
                    .unwrap()
                    .resources(&state.tick)
            })
        }
    }
    fn copper_ore(&mut self) -> WakeHandle<Bundle<CopperOre, 1>> {
        if self
            .resources
            .copper_territory
            .as_ref()
            .unwrap()
            .num_miners()
            == 0
        {
            self.hand_mine(|r| r.copper_territory.as_mut().unwrap())
        } else {
            self.wait_for_resource(|state| {
                state
                    .resources
                    .copper_territory
                    .as_mut()
                    .unwrap()
                    .resources(&state.tick)
            })
        }
    }

    fn iron<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Iron, COUNT>> {
        self.multiple(|state| {
            if let Ok(x) = state.resources.resource_store.get().bundle() {
                return state.nowait(x);
            } else {
                let ore = state.iron_ore();
                state.then(ore, |state, ore| {
                    state.craft1(ore, |resources| &mut resources.iron_furnace)
                })
            }
        })
    }
    fn copper<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Copper, COUNT>> {
        self.multiple(|state| {
            let ore = state.copper_ore();
            state.then(ore, |state, ore| {
                state.craft1(ore, |resources| &mut resources.copper_furnace)
            })
        })
    }
    fn steel<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Steel, COUNT>> {
        self.multiple(|state| {
            let iron = state.iron();
            state.then(iron, |state, ore| {
                state.craft1(ore, |resources| &mut resources.steel_furnace)
            })
        })
    }

    fn add_furnace<R: FurnaceRecipe + Any>(
        &mut self,
        r: R,
        f: fn(&mut Resources) -> &mut MachineStorage<Furnace<R>>,
    ) -> WakeHandle<()> {
        self.add_machine(f, |state| {
            let iron = state.iron();
            state.map(iron, |state, iron| Furnace::build(&state.tick, r, iron))
        })
    }

    fn add_miner<R: ResourceType + Any>(
        &mut self,
        f: fn(&mut Resources) -> &mut Option<Territory<R>>,
    ) -> WakeHandle<()> {
        let iron = self.iron();
        let copper = self.copper();
        let both = self.pair(iron, copper);
        self.map(both, move |state, (iron, copper)| {
            let miner = Miner::build(iron, copper);
            f(&mut state.resources)
                .as_mut()
                .unwrap()
                .add_miner(&state.tick, miner)
                .unwrap();
        })
    }

    fn copper_wire<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<CopperWire, COUNT>> {
        self.multiple(|state| {
            let copper = state.copper();
            state.then(copper, |state, copper| {
                if state.resources.copper_wire_assembler.is_present() {
                    state.craft1(copper, |resources| &mut resources.copper_wire_assembler)
                } else {
                    state.hand_craft::<_, CopperWireRecipe, _>((copper,))
                }
            })
        })
    }

    fn add_assembler<R: AssemblerRecipe + Any>(
        &mut self,
        r: R,
        f: fn(&mut Resources) -> &mut MachineStorage<Assembler<R>>,
    ) -> WakeHandle<()> {
        self.add_machine(f, |state| {
            let iron = state.iron();
            let copper_wire = state.copper_wire();
            let both = state.pair(copper_wire, iron);
            state.map(both, |state, (copper_wire, iron)| {
                Assembler::build(&state.tick, r, copper_wire, iron)
            })
        })
    }

    fn add_lab<T: Technology + Any>(
        &mut self,
        get_tech: fn(&Resources) -> &Option<T>,
        f: fn(&mut Resources) -> &mut MachineStorage<Lab<T>>,
    ) -> WakeHandle<()> {
        self.add_machine(f, |state| {
            let iron = state.iron();
            let copper = state.copper();
            let tech_ready = state.wait_until(move |state| get_tech(&state.resources).is_some());
            let triple = state.triple(tech_ready, iron, copper);
            state.map(triple, move |state, (_, iron, copper)| {
                let tech = get_tech(&state.resources).as_ref().unwrap();
                Lab::build(&state.tick, tech, iron, copper)
            })
        })
    }

    fn circuit<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<ElectronicCircuit, COUNT>> {
        self.multiple(|state| {
            let iron = state.iron();
            let copper_wire = state.copper_wire();
            let both = state.pair(copper_wire, iron);
            state.then(both, |state, (copper_wire, iron)| {
                state.craft((iron, copper_wire), |r| &mut r.elec_circuit_assembler)
            })
        })
    }

    fn red_science<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<RedScience, COUNT>> {
        self.multiple(|state| {
            let iron = state.iron();
            let circuit = state.circuit();
            let both = state.pair(iron, circuit);
            state.then(both, |state, (iron, circuit)| {
                state.hand_craft::<_, RedScienceRecipe, _>((iron, circuit))
            })
        })
    }

    fn red_science_tech<const COUNT: u32, T: Technology + Any>(
        &mut self,
        lab: fn(&mut Resources) -> &mut MachineStorage<Lab<T>>,
    ) -> WakeHandle<Bundle<ResearchPoint<T>, COUNT>>
    where
        <TechRecipe<T> as Recipe>::Inputs: MachineInputs,
        TechRecipe<T>: ConstRecipe<
                1,
                BundledInputs = (Bundle<RedScience, 1>,),
                BundledOutputs = (Bundle<ResearchPoint<T>, 1>,),
            >,
    {
        self.multiple(move |state| {
            let science = state.red_science();
            state.then(science, move |state, science| state.craft1(science, lab))
        })
    }

    fn points<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Point, COUNT>> {
        self.multiple(|state| {
            let circuit = state.circuit();
            let steel = state.steel();
            let both = state.pair(circuit, steel);
            state.then(both, |state, (circuit, steel)| {
                state.craft((circuit, steel), |r| &mut r.points_assembler)
            })
        })
    }

    fn play(mut self) -> (Tick, Bundle<Point, 200>) {
        let h = self.add_furnace(IronSmelting, |r| &mut r.iron_furnace);
        self.complete(h);

        let h = self.add_furnace(CopperSmelting, |r| &mut r.copper_furnace);
        self.complete(h);

        self.add_miner(|r| &mut r.iron_territory);
        self.add_miner(|r| &mut r.copper_territory);

        self.add_furnace(IronSmelting, |r| &mut r.iron_furnace);
        self.add_furnace(IronSmelting, |r| &mut r.iron_furnace);
        self.add_furnace(CopperSmelting, |r| &mut r.copper_furnace);

        let h = self.add_assembler(CopperWireRecipe, |r| &mut r.copper_wire_assembler);
        self.complete(h);

        self.add_assembler(ElectronicCircuitRecipe, |r| &mut r.elec_circuit_assembler);

        self.add_lab(|r| &r.steel_technology, |r| &mut r.steel_lab);

        // self.add_miner(|r| &mut r.iron_territory);
        // self.add_furnace(IronSmelting, |r| &mut r.iron_furnace);
        // self.add_miner(|r| &mut r.copper_territory);
        // self.add_furnace(CopperSmelting, |r| &mut r.copper_furnace);

        // self.add_assembler(CopperWireRecipe, |r| &mut r.copper_wire_assembler);
        // self.add_assembler(ElectronicCircuitRecipe, |r| &mut r.elec_circuit_assembler);

        let research_points = self.red_science_tech(|r| &mut r.steel_lab);
        self.map(research_points, |state, research_points| {
            let steel_tech = state.resources.steel_technology.take().unwrap();
            let (steel_smelting, points_tech) = steel_tech.research(research_points);
            state.resources.points_lab = state
                .resources
                .steel_lab
                .take_map(|lab| lab.change_technology(&points_tech).unwrap());
            state.resources.steel_smelting = Some(steel_smelting);
            state.resources.points_technology = Some(points_tech);
        });

        let steel_smelting = self.wait_for(|st| st.resources.steel_smelting);
        self.then(steel_smelting, |state, steel_smelting| {
            state.add_furnace(steel_smelting, |r| &mut r.steel_furnace)
        });

        // self.add_lab(|r| &r.points_technology, |r| &mut r.points_lab);

        let research_points = self.red_science_tech(|r| &mut r.points_lab);
        self.map(research_points, |state, research_points| {
            let points_tech = state.resources.points_technology.take().unwrap();
            let points_recipe = points_tech.research(research_points);
            state.resources.points_recipe = Some(points_recipe);
        });

        let points_recipe = self.wait_for(|st| st.resources.points_recipe);
        self.then(points_recipe, |state, points_recipe| {
            state.add_assembler(points_recipe, |r| &mut r.points_assembler)
        });

        // self.add_miner(|r| &mut r.iron_territory);
        // self.add_furnace(IronSmelting, |r| &mut r.iron_furnace);

        let _points = self.points::<10>();
        self.complete_all();
        todo!("WIP: {}", self.tick.cur())
        // let points = self.complete(points);
        // (self.tick, points)
    }
}
