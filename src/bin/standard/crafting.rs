use std::{any::Any, marker::PhantomData};

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
    territory::{Miner, Territory},
};
use rustorio_engine::research::TechRecipe;

use crate::{
    GameState,
    machine::{
        HandCrafter, Machine, MultiMachine, OnceMaker, Priority, Producer, ProducerWithQueue,
        TheFirstTime,
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

/// A tuple of `Bundle<R, N>`.
pub trait MultiBundle: Sized {
    /// The corresponding tuple of `Resource<R>`.
    type AsResource;

    /// Count the number of bundle tuples available in the given resource tuple.
    fn bundle_count(res: &Self::AsResource) -> u32;
    /// Add the bundle tuple to the resource tuple.
    fn add(res: &mut Self::AsResource, bundle: Self);
    /// Pop a bundle tuple from a resource tuple, if there are enough resources.
    fn bundle(res: &mut Self::AsResource) -> Option<Self>;
}

impl<R1: ResourceType, const N1: u32> MultiBundle for (Bundle<R1, N1>,) {
    type AsResource = (Resource<R1>,);

    fn bundle_count(res: &Self::AsResource) -> u32 {
        res.0.amount() / N1
    }
    fn add(res: &mut Self::AsResource, bundle: Self) {
        res.0 += bundle.0;
    }
    fn bundle(res: &mut Self::AsResource) -> Option<Self> {
        Some((res.0.bundle().ok()?,))
    }
}
impl<R1: ResourceType, const N1: u32, R2: ResourceType, const N2: u32> MultiBundle
    for (Bundle<R1, N1>, Bundle<R2, N2>)
{
    type AsResource = (Resource<R1>, Resource<R2>);

    fn bundle_count(res: &Self::AsResource) -> u32 {
        std::cmp::min(res.0.amount() / N1, res.1.amount() / N2)
    }
    fn add(res: &mut Self::AsResource, bundle: Self) {
        res.0 += bundle.0;
        res.1 += bundle.1;
    }
    fn bundle(res: &mut Self::AsResource) -> Option<Self> {
        if res.0.amount() >= N1 && res.1.amount() >= N2 {
            Some((res.0.bundle().ok()?, res.1.bundle().ok()?))
        } else {
            None
        }
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
    type BundledInputs_: MultiBundle<AsResource = Self::Inputs>;
    type BundledOutputs_: MultiBundle<AsResource = Self::Outputs>;
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
    type BundledInputs: MultiBundle<AsResource = Self::Inputs>;
    type BundledOutputs: MultiBundle<AsResource = Self::Outputs>;
}
impl<R: Recipe + InputN + Any + ConstRecipeImpl<{ R::INPUT_N }>> ConstRecipe for R {
    type BundledInputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledInputs_;
    type BundledOutputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledOutputs_;
}

pub trait SingleOutputMachine:
    Machine<Recipe: ConstRecipe<BundledOutputs = (Self::Output,)>>
{
    type Output;
}
impl<M: Machine<Recipe: ConstRecipe<BundledOutputs = (O,)>>, O> SingleOutputMachine for M {
    type Output = O;
}

pub trait SingleOutputProducer:
    Producer<Output = (<Self as SingleOutputProducer>::Output,)>
{
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
impl<A: Makeable> Makeable for (A,) {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let h = A::make(state, p);
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
impl<const N: usize, T: Makeable> Makeable for [T; N] {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let handles = (0..N).map(|_| T::make(state, p)).collect_vec();
        let h = state.collect(handles);
        state.map(h, |_, v| v.try_into().ok().unwrap())
    }
}

/// Items that can be produced from a given makeable input.
pub trait InputMakeable: Sized + Any {
    type Input: Makeable;

    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        let inputs = state.make(p);
        state.map(inputs, move |state, inputs| {
            Self::make_from_input(state, inputs)
        })
    }

    fn make_from_input(state: &mut GameState, input: Self::Input) -> Self;
}
impl<R: InputMakeable> Makeable for R {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        <Self as InputMakeable>::make(state, p)
    }
}

impl InputMakeable for Miner {
    type Input = (Bundle<Iron, 10>, Bundle<Copper, 5>);

    fn make_from_input(_state: &mut GameState, (iron, copper): Self::Input) -> Self {
        Miner::build(iron, copper)
    }
}
impl<R> InputMakeable for Furnace<R>
where
    R: FurnaceRecipe + Recipe + Makeable,
{
    type Input = (R, Bundle<Iron, 10>);

    fn make_from_input(state: &mut GameState, (r, iron): Self::Input) -> Self {
        Furnace::build(&state.tick, r, iron)
    }
}
impl<R> InputMakeable for Assembler<R>
where
    R: AssemblerRecipe + Recipe + Makeable,
{
    type Input = (R, Bundle<Iron, 6>, Bundle<CopperWire, 12>);

    fn make_from_input(state: &mut GameState, (r, iron, copper_wire): Self::Input) -> Self {
        Assembler::build(&state.tick, r, copper_wire, iron)
    }
}
impl<T: Technology> InputMakeable for Lab<T>
where
    TechAvailable<T>: Makeable,
{
    type Input = (Bundle<Iron, 20>, Bundle<Copper, 15>, TechAvailable<T>);

    fn make_from_input(state: &mut GameState, (iron, copper, _): Self::Input) -> Self {
        let tech = state.resources.tech().as_ref().unwrap();
        Lab::build(&state.tick, tech, iron, copper)
    }
}

pub struct TechAvailable<T: Technology>(PhantomData<T>);

impl InputMakeable for TechAvailable<SteelTechnology> {
    type Input = ();
    fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
        Self(PhantomData)
    }
}
impl InputMakeable for TechAvailable<PointsTechnology> {
    // The steel smelting recipe is because it also sets up the points tech.
    type Input = SteelSmelting;
    fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
        Self(PhantomData)
    }
}

impl InputMakeable for TheFirstTime<SteelSmelting> {
    type Input = Bundle<ResearchPoint<SteelTechnology>, 20>;

    fn make_from_input(state: &mut GameState, research_points: Self::Input) -> Self {
        let steel_tech: SteelTechnology = state.resources.tech().take().unwrap();
        let (steel_smelting, points_tech) = steel_tech.research(research_points);
        let pqw = state.resources.machine::<Lab<SteelTechnology>>();
        assert_eq!(pqw.queue.len(), 0);
        let lab = pqw
            .producer
            .take_map(|lab| lab.change_technology(&points_tech).unwrap());
        *state.resources.machine() = ProducerWithQueue::new(lab);
        *state.resources.tech() = Some(points_tech);
        TheFirstTime(steel_smelting)
    }
}
impl Makeable for SteelSmelting {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        // If we let scaling up happen automatically, we apparently lose ticks :(
        state.scale_up::<OnceMaker<Self>>(p);
        state.feed_producer::<OnceMaker<Self>>(p)
    }
}

impl InputMakeable for TheFirstTime<PointRecipe> {
    type Input = Bundle<ResearchPoint<PointsTechnology>, 50>;

    fn make_from_input(state: &mut GameState, research_points: Self::Input) -> Self {
        let points_tech: PointsTechnology = state.resources.tech().take().unwrap();
        let points_recipe = points_tech.research(research_points);
        TheFirstTime(points_recipe)
    }
}
impl Makeable for PointRecipe {
    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        // If we let scaling up happen automatically, we apparently lose ticks :(
        state.scale_up::<OnceMaker<Self>>(p);
        state.feed_producer::<OnceMaker<Self>>(p)
    }
}

impl<const AMOUNT: u32, R: ProducerMakeable> InputMakeable for Bundle<R, AMOUNT>
where
    [(); (AMOUNT / <R::Producer as SingleOutputProducer>::Output::AMOUNT) as usize]:,
{
    type Input = [<R::Producer as Producer>::Input;
        (AMOUNT / <R::Producer as SingleOutputProducer>::Output::AMOUNT) as usize];

    fn make(state: &mut GameState, p: Priority) -> WakeHandle<Self> {
        if let Ok(x) = state.resources.resource().bundle() {
            return state.nowait(x);
        }
        // We cleverly don't fetch the whole input at once. Instead, as soon as the first input
        // bundle arrives we feed it to the producer.
        state.multiple(|state| {
            let out = state.feed_producer::<R::Producer>(p);
            state.map(out, |_, out| out.0)
        })
    }

    // We never call this because we don't want to fetch the whole input at once.
    fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
        unreachable!()
    }
}

/// Items that can be produced by producers. This is the heart of the crafting logic.
/// This maker fetches the required inputs, gives them to the producer, then waits for the producer
/// to produce its output.
pub trait ProducerMakeable: ResourceType + Sized + Any {
    type Producer: SingleOutputProducer<Input: Makeable, Output: IsBundle<Resource = Self>>;
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
    type Producer = MultiMachine<R::Machine>;
}

pub trait MachineMakeable: ResourceType + Any + Sized {
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
impl<T: ConstMakeable + Any> InputMakeable for T {
    type Input = ();
    fn make_from_input(_state: &mut GameState, _: ()) -> Self {
        Self::MAKE
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

/// Compute the cost of a given item in terms of another one.
pub trait CostIn<O> {
    const COST: u32;
}
impl<O, T> CostIn<O> for T {
    default const COST: u32 = <T as InputCost<O>>::COST + <T as SelfCost<O>>::COST;
}
impl<O> CostIn<O> for () {
    const COST: u32 = 0;
}
impl<O, A: CostIn<O>> CostIn<O> for (A,) {
    const COST: u32 = A::COST;
}
impl<O, A: CostIn<O>, B: CostIn<O>> CostIn<O> for (A, B) {
    const COST: u32 = A::COST + B::COST;
}
impl<O, A: CostIn<O>, B: CostIn<O>, C: CostIn<O>> CostIn<O> for (A, B, C) {
    const COST: u32 = A::COST + B::COST + C::COST;
}
impl<O, T: ConstMakeable> CostIn<O> for T {
    const COST: u32 = 0;
}
impl<O, R: CostIn<O>, const N: usize> CostIn<O> for [R; N] {
    const COST: u32 = N as u32 * R::COST;
}

trait SelfCost<A> {
    const COST: u32;
}
impl<A, B> SelfCost<A> for B {
    default const COST: u32 = 0;
}
impl<A> SelfCost<A> for A {
    const COST: u32 = 1;
}
impl<A: ResourceType, const N: u32> SelfCost<A> for Bundle<A, N> {
    const COST: u32 = N;
}

trait InputCost<A> {
    const COST: u32;
}
impl<O, T> InputCost<O> for T {
    default const COST: u32 = 0;
}
impl<O, T> InputCost<O> for T
where
    Self: InputMakeable<Input: CostIn<O>>,
{
    const COST: u32 = <<Self as InputMakeable>::Input as CostIn<O>>::COST;
}

const _: () = {
    assert!(<IronOre as CostIn<IronOre>>::COST == 1);
    assert!(<Bundle<Iron, 1> as InputCost<IronOre>>::COST == 1);
    assert!(<Miner as CostIn<IronOre>>::COST == 10);
    assert!(<Miner as CostIn<Bundle<IronOre, 1>>>::COST == 10);
    assert!(<Miner as CostIn<CopperOre>>::COST == 5);
    assert!(<Furnace<IronSmelting> as CostIn<IronOre>>::COST == 10);
    assert!(<Iron as CostIn<Iron>>::COST == 1);
};

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
