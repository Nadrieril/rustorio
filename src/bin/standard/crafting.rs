use std::any::Any;

use itertools::Itertools;
use rustorio::{
    Bundle, Recipe, ResearchPoint, Resource, ResourceType, Technology,
    buildings::{Assembler, Furnace, Lab},
    recipes::{
        AssemblerRecipe, CopperSmelting, CopperWireRecipe, ElectronicCircuitRecipe, FurnaceRecipe,
        IronSmelting, PointRecipe, RedScienceRecipe, SteelSmelting,
    },
    research::{PointsTechnology, RedScience, SteelTechnology},
    resources::{Copper, CopperOre, CopperWire, ElectronicCircuit, Iron, IronOre, Point, Steel},
    territory::Territory,
};
use rustorio_engine::research::TechRecipe;

use crate::{
    GameState,
    machine::{
        HandCrafter, Machine, MachineStorage, OnceMaker, Priority, Producer, ProducerWithQueue,
    },
    scheduler::WakeHandle,
};

pub trait IsBundle {
    const AMOUNT: u32;
    type Resource: ResourceType;
    fn to_resource(self) -> Resource<Self::Resource>;
}
impl<const AMOUNT: u32, R: ResourceType> IsBundle for Bundle<R, AMOUNT> {
    const AMOUNT: u32 = AMOUNT;
    type Resource = R;
    fn to_resource(self) -> Resource<Self::Resource> {
        self.to_resource()
    }
}
impl<const AMOUNT: u32, R: ResourceType> IsBundle for (Bundle<R, AMOUNT>,) {
    const AMOUNT: u32 = AMOUNT;
    type Resource = R;
    fn to_resource(self) -> Resource<Self::Resource> {
        self.0.to_resource()
    }
}

// Const fns because direct field access is not allowed in const exprs.
pub const fn tup1_field0<A: Copy>(x: (A,)) -> A {
    x.0
}
pub const fn tup2_field0<A: Copy, B: Copy>(x: (A, B)) -> A {
    x.0
}
pub const fn tup2_field1<A: Copy, B: Copy>(x: (A, B)) -> B {
    x.1
}

/// Trait to compute statically-counted inputs and outputs. The const generic is needed because the
/// impls would otherwise be considered to overlap.
pub trait ConstRecipeImpl<const INPUT_N: u32>: Recipe {
    type BundledInputs_;
    type BundledOutputs_;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load(input: &Self::Inputs) -> u32;
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs_);
    /// Used to load-balance across machines of the same type.
    fn pop_inputs(from: &mut Self::Inputs) -> Option<Self::BundledInputs_>;
    /// Used when we handcrafted some values, to have somewhere to store them.
    fn add_outputs(to: &mut Self::Outputs, o: Self::BundledOutputs_);
    fn pop_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs_>;
}

impl<R, I, O> ConstRecipeImpl<1> for R
where
    I: ResourceType,
    O: ResourceType,
    R: Recipe<Inputs = (Resource<I>,), InputAmountsType = (u32,)>,
    R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
    [(); { tup1_field0(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup1_field0(R::OUTPUT_AMOUNTS) } as usize]:,
{
    type BundledInputs_ = (Bundle<I, { tup1_field0(R::INPUT_AMOUNTS) }>,);
    type BundledOutputs_ = (Bundle<O, { tup1_field0(R::OUTPUT_AMOUNTS) }>,);
    fn input_load(input: &Self::Inputs) -> u32 {
        input.0.amount() / R::INPUT_AMOUNTS.0
    }
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs_) {
        to.0.add(i.0);
    }
    fn pop_inputs(from: &mut Self::Inputs) -> Option<Self::BundledInputs_> {
        Some((from.0.bundle().ok()?,))
    }
    fn add_outputs(to: &mut Self::Outputs, o: Self::BundledOutputs_) {
        to.0.add(o.0);
    }
    fn pop_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs_> {
        Some((from.0.bundle().ok()?,))
    }
}

impl<R, I1, I2, O> ConstRecipeImpl<2> for R
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
    type BundledInputs_ = (
        Bundle<I1, { tup2_field0(R::INPUT_AMOUNTS) }>,
        Bundle<I2, { tup2_field1(R::INPUT_AMOUNTS) }>,
    );
    type BundledOutputs_ = (Bundle<O, { tup1_field0(R::OUTPUT_AMOUNTS) }>,);
    fn input_load(input: &Self::Inputs) -> u32 {
        input.0.amount() / R::INPUT_AMOUNTS.0
    }
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs_) {
        to.0.add(i.0);
        to.1.add(i.1);
    }
    fn pop_inputs(from: &mut Self::Inputs) -> Option<Self::BundledInputs_> {
        if from.0.amount() >= R::INPUT_AMOUNTS.0 && from.1.amount() >= R::INPUT_AMOUNTS.1 {
            Some((from.0.bundle().ok()?, from.1.bundle().ok()?))
        } else {
            None
        }
    }
    fn add_outputs(to: &mut Self::Outputs, o: Self::BundledOutputs_) {
        to.0.add(o.0);
    }
    fn pop_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs_> {
        Some((from.0.bundle().ok()?,))
    }
}

/// Provides the `INPUT_N` to pass to `ConstRecipe`. This is weird af but seems to work.
/// Can't define generic impls for this of course, since rustc considers them to overlap.
pub trait InputN {
    const INPUT_N: u32;
}
impl InputN for IronSmelting {
    const INPUT_N: u32 = 1;
}
impl InputN for CopperSmelting {
    const INPUT_N: u32 = 1;
}
impl InputN for SteelSmelting {
    const INPUT_N: u32 = 1;
}
impl InputN for CopperWireRecipe {
    const INPUT_N: u32 = 1;
}
impl InputN for TechRecipe<SteelTechnology> {
    const INPUT_N: u32 = 1;
}
impl InputN for TechRecipe<PointsTechnology> {
    const INPUT_N: u32 = 1;
}
impl InputN for ElectronicCircuitRecipe {
    const INPUT_N: u32 = 2;
}
impl InputN for RedScienceRecipe {
    const INPUT_N: u32 = 2;
}
impl InputN for PointRecipe {
    const INPUT_N: u32 = 2;
}

/// Trait to compute statically-counted inputs and outputs.
pub trait ConstRecipe: Recipe + InputN + Any {
    type BundledInputs;
    type BundledOutputs;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load(input: &Self::Inputs) -> u32;
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs);
    /// Used to load-balance across machines of the same type.
    fn pop_inputs(from: &mut Self::Inputs) -> Option<Self::BundledInputs>;
    /// Used when we handcrafted some values, to have somewhere to store them.
    fn add_outputs(to: &mut Self::Outputs, o: Self::BundledOutputs);
    fn pop_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs>;
}
impl<R: Recipe + InputN + Any + ConstRecipeImpl<{ R::INPUT_N }>> ConstRecipe for R {
    type BundledInputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledInputs_;
    type BundledOutputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledOutputs_;
    fn input_load(input: &Self::Inputs) -> u32 {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::input_load(input)
    }
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs) {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::add_inputs(to, i);
    }
    fn pop_inputs(from: &mut Self::Inputs) -> Option<Self::BundledInputs> {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::pop_inputs(from)
    }
    fn add_outputs(to: &mut Self::Outputs, o: Self::BundledOutputs) {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::add_outputs(to, o)
    }
    fn pop_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs> {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::pop_outputs(from)
    }
}

trait SingleOutputMachine: Machine<Recipe: ConstRecipe<BundledOutputs = (Self::Output,)>> {
    type Output;
}
impl<M: Machine<Recipe: ConstRecipe<BundledOutputs = (O,)>>, O> SingleOutputMachine for M {
    type Output = O;
}

trait SingleOutputProducer: Producer<Output = (<Self as SingleOutputProducer>::Output,)> {
    type Output;
}
impl<P: Producer<Output = (O,)>, O> SingleOutputProducer for P {
    type Output = O;
}

pub trait Makeable: Any + Sized {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self>;
}
impl Makeable for () {
    fn make(state: &mut GameState, _p: Priority) -> WakeHandle<Self> {
        state.nowait(())
    }
}
impl<T: Makeable> Makeable for (T,) {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let h = T::make(state, p);
        state.map(h, |_, v| (v,))
    }
}
impl<A: Makeable, B: Makeable> Makeable for (A, B) {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let a = A::make(state, p);
        let b = B::make(state, p);
        state.pair(a, b)
    }
}
impl<A: Makeable, B: Makeable, C: Makeable> Makeable for (A, B, C) {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let a = A::make(state, p);
        let b = B::make(state, p);
        let c = C::make(state, p);
        state.triple(a, b, c)
    }
}

impl Makeable for SteelSmelting {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        if state
            .resources
            .producers
            .once_maker::<Self>()
            .producer
            .start()
        {
            let research_points = state.make(p);
            state.map(research_points, |state, research_points| {
                let steel_tech = state.resources.steel_technology.take().unwrap();
                let (steel_smelting, points_tech) = steel_tech.research(research_points);
                let pqw = state.resources.producers.machine::<Lab<SteelTechnology>>();
                assert_eq!(pqw.queue.len(), 0);
                let lab = pqw
                    .producer
                    .take_map(|lab| lab.change_technology(&points_tech).unwrap());
                *state.resources.producers.machine() = ProducerWithQueue::new(lab);
                state
                    .resources
                    .producers
                    .once_maker()
                    .producer
                    .set(steel_smelting);
                state.resources.points_technology = Some(points_tech);
            });
        }
        state.wait_for_producer_output::<OnceMaker<Self>>(p)
    }
}
impl Makeable for PointRecipe {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        if state
            .resources
            .producers
            .once_maker::<Self>()
            .producer
            .start()
        {
            let research_points = state.make(p);
            state.map(research_points, |state, research_points| {
                let points_tech = state.resources.points_technology.take().unwrap();
                let points_recipe = points_tech.research(research_points);
                state
                    .resources
                    .producers
                    .once_maker()
                    .producer
                    .set(points_recipe);
            });
        }
        state.wait_for_producer_output::<OnceMaker<Self>>(p)
    }
}

impl<R> Makeable for Furnace<R>
where
    R: FurnaceRecipe + Recipe + Makeable,
{
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let inputs = state.make(p);
        state.map(inputs, |state, (r, iron)| {
            Furnace::build(&state.tick, r, iron)
        })
    }
}
impl<R> Makeable for Assembler<R>
where
    R: AssemblerRecipe + Recipe + Makeable,
{
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let inputs = state.make(p);
        state.map(inputs, |state, (r, copper_wire, iron)| {
            Assembler::build(&state.tick, r, copper_wire, iron)
        })
    }
}
impl Makeable for Lab<SteelTechnology> {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let inputs = state.make(p);
        state.map(inputs, move |state, (iron, copper)| {
            let tech = state.resources.steel_technology.as_ref().unwrap();
            Lab::build(&state.tick, tech, iron, copper)
        })
    }
}
impl Makeable for Lab<PointsTechnology> {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let inputs = state.make(p);
        state.map(inputs, move |state, (iron, copper, steel_smelting)| {
            // Wait for the steel smelting recipe, because it also sets up the points tech.
            let _: SteelSmelting = steel_smelting;
            let tech = state.resources.points_technology.as_ref().unwrap();
            Lab::build(&state.tick, tech, iron, copper)
        })
    }
}

impl<const AMOUNT: u32, R: ProducerMakeable> Makeable for Bundle<R, AMOUNT> {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        R::make_many(state, p)
    }
}

/// Items that can be produced by producers. This is the heart of the crafting logic.
/// This maker fetches the required inputs, gives them to the producer, then waits for the producer
/// to produce its output.
trait ProducerMakeable: ResourceType + Sized + Any {
    type Producer: SingleOutputProducer<Input: Makeable, Output: IsBundle<Resource = Self>>;

    fn make_one(
        state: &mut GameState,
        p: Priority,
    ) -> WakeHandle<<Self::Producer as SingleOutputProducer>::Output> {
        let inputs = state.make(p);
        let out = state.then(inputs, move |state, inputs| {
            Self::Producer::get_ref(&mut state.resources)
                .producer
                .add_inputs(&state.tick, inputs);
            state.wait_for_producer_output::<Self::Producer>(p)
        });
        state.map(out, |_, out| out.0)
    }
    fn make_many<const AMOUNT: u32>(
        state: &mut GameState,
        p: Priority,
    ) -> WakeHandle<Bundle<Self, AMOUNT>> {
        if let Ok(x) = state.resources.resource_store.get().bundle() {
            state.nowait(x)
        } else {
            state.multiple(|state| Self::make_one(state, p))
        }
    }
}

impl ProducerMakeable for IronOre {
    type Producer = Territory<Self>;
}
impl ProducerMakeable for CopperOre {
    type Producer = Territory<Self>;
}
impl ProducerMakeable for RedScience {
    type Producer = HandCrafter<RedScienceRecipe>;
}
impl<R: MachineMakeable> ProducerMakeable for R {
    type Producer = MachineStorage<R::Machine>;
}

trait MachineMakeable: ResourceType + Any + Sized {
    type Machine: Machine<Recipe: ConstRecipe<BundledInputs: Makeable>>
        + SingleOutputMachine<Output: IsBundle<Resource = Self>>
        + Makeable;
}

impl MachineMakeable for Iron {
    type Machine = Furnace<IronSmelting>;
}
impl MachineMakeable for Copper {
    type Machine = Furnace<CopperSmelting>;
}
impl MachineMakeable for Steel {
    type Machine = Furnace<SteelSmelting>;
}
impl MachineMakeable for CopperWire {
    type Machine = Assembler<CopperWireRecipe>;
}
impl MachineMakeable for ElectronicCircuit {
    type Machine = Assembler<ElectronicCircuitRecipe>;
}
impl MachineMakeable for Point {
    type Machine = Assembler<PointRecipe>;
}
impl<T: Technology + Any> MachineMakeable for ResearchPoint<T>
where
    TechRecipe<T>: ConstRecipe<BundledInputs: Makeable, BundledOutputs = (Bundle<Self, 1>,)>,
    Lab<T>: Makeable,
{
    type Machine = Lab<T>;
}

trait ConstMakeable {
    const MAKE: Self;
}
impl<T: ConstMakeable + Any> Makeable for T {
    fn make(state: &mut GameState, _p: Priority) -> WakeHandle<Self> {
        state.nowait(Self::MAKE)
    }
}

impl ConstMakeable for IronSmelting {
    const MAKE: Self = IronSmelting;
}
impl ConstMakeable for CopperSmelting {
    const MAKE: Self = CopperSmelting;
}
impl ConstMakeable for CopperWireRecipe {
    const MAKE: Self = CopperWireRecipe;
}
impl ConstMakeable for ElectronicCircuitRecipe {
    const MAKE: Self = ElectronicCircuitRecipe;
}

impl GameState {
    pub fn make<T: Makeable>(&mut self, p: Priority) -> WakeHandle<T> {
        T::make(self, p)
    }

    /// Given a producer of a single bundle of an item, make a producer of a larger bundle.
    pub fn multiple<const COUNT: u32, R, B>(
        &mut self,
        f: impl Fn(&mut GameState) -> WakeHandle<B>,
    ) -> WakeHandle<Bundle<R, COUNT>>
    where
        R: ResourceType + Any,
        B: IsBundle<Resource = R> + Any,
    {
        assert_eq!(COUNT.rem_euclid(B::AMOUNT), 0);
        let singles = (0..COUNT / B::AMOUNT).map(|_| f(self)).collect_vec();
        let sum = self.collect_sum(singles);
        self.map(sum, |_, mut sum| sum.bundle().unwrap())
    }
}
