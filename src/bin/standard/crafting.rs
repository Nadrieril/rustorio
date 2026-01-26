use std::any::Any;

use itertools::Itertools;
use rustorio::{
    Bundle, HandRecipe, Recipe, ResearchPoint, Resource, ResourceType, Technology,
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
    GameState, Resources,
    machine::{Machine, MachineInputs, MachineSlot},
    scheduler::WakeHandle,
};

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
    type BundledInputs;
    type BundledOutputs;
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs);
    fn get_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs>;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load(input: &Self::Inputs) -> u32;
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
impl InputN for PointRecipe {
    const INPUT_N: u32 = 2;
}
impl InputN for ElectronicCircuitRecipe {
    const INPUT_N: u32 = 2;
}

/// Trait to compute statically-counted inputs and outputs.
pub trait ConstRecipe: Recipe + InputN {
    type BundledInputs;
    type BundledOutputs;
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs);
    fn get_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs>;
    /// Count the number of recipe instances left in this input bundle.
    fn input_load(input: &Self::Inputs) -> u32;
}
impl<R: Recipe + InputN + ConstRecipeImpl<{ R::INPUT_N }>> ConstRecipe for R {
    type BundledInputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledInputs;
    type BundledOutputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledOutputs;
    fn add_inputs(to: &mut Self::Inputs, i: Self::BundledInputs) {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::add_inputs(to, i);
    }
    fn get_outputs(from: &mut Self::Outputs) -> Option<Self::BundledOutputs> {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::get_outputs(from)
    }
    fn input_load(input: &Self::Inputs) -> u32 {
        <R as ConstRecipeImpl<{ R::INPUT_N }>>::input_load(input)
    }
}

pub trait Makeable: Any + Sized {
    fn make(state: &mut GameState) -> WakeHandle<Self>;
}
impl<T: Makeable> Makeable for (T,) {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let h = T::make(state);
        state.map(h, |_, v| (v,))
    }
}
impl<A: Makeable, B: Makeable> Makeable for (A, B) {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let a = A::make(state);
        let b = B::make(state);
        state.pair(a, b)
    }
}
impl<A: Makeable, B: Makeable, C: Makeable> Makeable for (A, B, C) {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let a = A::make(state);
        let b = B::make(state);
        let c = C::make(state);
        state.triple(a, b, c)
    }
}

impl<M: Machine + Any> Makeable for MachineSlot<M> {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        state.wait_for(move |s| s.resources.machine_store.for_type::<M>().request(&s.tick))
    }
}

impl Makeable for SteelSmelting {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        if let Some(recipe) = state.resources.steel_smelting {
            state.nowait(recipe)
        } else {
            let research_points = state.make();
            state.map(research_points, |state, research_points| {
                let steel_tech = state.resources.steel_technology.take().unwrap();
                let (steel_smelting, points_tech) = steel_tech.research(research_points);
                let lab = state
                    .resources
                    .machine_store
                    .for_type::<Lab<SteelTechnology>>()
                    .take_map(|lab| lab.change_technology(&points_tech).unwrap());
                *state.resources.machine_store.for_type() = lab;
                state.resources.steel_smelting = Some(steel_smelting);
                state.resources.points_technology = Some(points_tech);
                steel_smelting
            })
        }
    }
}
impl Makeable for PointRecipe {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        if let Some(recipe) = state.resources.points_recipe {
            state.nowait(recipe)
        } else {
            let research_points = state.make();
            state.map(research_points, |state, research_points| {
                let points_tech = state.resources.points_technology.take().unwrap();
                let points_recipe = points_tech.research(research_points);
                state.resources.points_recipe = Some(points_recipe);
                points_recipe
            })
        }
    }
}

impl<R> Makeable for Furnace<R>
where
    R: FurnaceRecipe + Recipe<Inputs: MachineInputs> + Makeable,
{
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let inputs = state.make();
        state.map(inputs, |state, (r, iron)| {
            Furnace::build(&state.tick, r, iron)
        })
    }
}
impl<R> Makeable for Assembler<R>
where
    R: AssemblerRecipe + Recipe<Inputs: MachineInputs> + Makeable,
{
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let inputs = state.make();
        state.map(inputs, |state, (r, copper_wire, iron)| {
            Assembler::build(&state.tick, r, copper_wire, iron)
        })
    }
}
impl Makeable for Lab<SteelTechnology> {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let inputs = state.make();
        state.map(inputs, move |state, (iron, copper)| {
            let tech = state.resources.steel_technology.as_ref().unwrap();
            Lab::build(&state.tick, tech, iron, copper)
        })
    }
}
impl Makeable for Lab<PointsTechnology> {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        let inputs = state.make();
        state.map(inputs, move |state, (iron, copper, steel_smelting)| {
            // Wait for the steel smelting recipe, because it also sets up the points tech.
            let _: SteelSmelting = steel_smelting;
            let tech = state.resources.points_technology.as_ref().unwrap();
            Lab::build(&state.tick, tech, iron, copper)
        })
    }
}

trait BundleMakeable: ResourceType + Any + Sized {
    fn craft_one(state: &mut GameState) -> WakeHandle<Bundle<Self, 1>>;
    fn craft_many<const AMOUNT: u32>(state: &mut GameState) -> WakeHandle<Bundle<Self, AMOUNT>> {
        if let Ok(x) = state.resources.resource_store.get().bundle() {
            state.nowait(x)
        } else {
            state.multiple(|state| Self::craft_one(state))
        }
    }
}
impl<const AMOUNT: u32, R: BundleMakeable> Makeable for Bundle<R, AMOUNT> {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
        R::craft_many(state)
    }
}

impl BundleMakeable for IronOre {
    fn craft_one(state: &mut GameState) -> WakeHandle<Bundle<Self, 1>> {
        if state
            .resources
            .iron_territory
            .as_ref()
            .unwrap()
            .num_miners()
            == 0
        {
            state.hand_mine(|r| r.iron_territory.as_mut().unwrap())
        } else {
            state.wait_for_resource(|state| {
                state
                    .resources
                    .iron_territory
                    .as_mut()
                    .unwrap()
                    .resources(&state.tick)
            })
        }
    }
}
impl BundleMakeable for CopperOre {
    fn craft_one(state: &mut GameState) -> WakeHandle<Bundle<Self, 1>> {
        if state
            .resources
            .copper_territory
            .as_ref()
            .unwrap()
            .num_miners()
            == 0
        {
            state.hand_mine(|r| r.copper_territory.as_mut().unwrap())
        } else {
            state.wait_for_resource(|state| {
                state
                    .resources
                    .copper_territory
                    .as_mut()
                    .unwrap()
                    .resources(&state.tick)
            })
        }
    }
}
impl BundleMakeable for CopperWire {
    fn craft_one(_state: &mut GameState) -> WakeHandle<Bundle<Self, 1>> {
        panic!("can't craft single copper wire")
    }
    fn craft_many<const AMOUNT: u32>(state: &mut GameState) -> WakeHandle<Bundle<Self, AMOUNT>> {
        state.multiple(|state| {
            state.make_then(|state, inputs| {
                if state
                    .resources
                    .machine_store
                    .for_type::<Assembler<CopperWireRecipe>>()
                    .is_present()
                {
                    state.craft::<Assembler<CopperWireRecipe>, _>(inputs)
                } else {
                    state.hand_craft::<_, CopperWireRecipe, _>(inputs)
                }
            })
        })
    }
}
impl BundleMakeable for RedScience {
    fn craft_one(state: &mut GameState) -> WakeHandle<Bundle<Self, 1>> {
        state.make_then(|state, inputs| state.hand_craft::<_, RedScienceRecipe, _>(inputs))
    }
}

trait MachineMakeable: ResourceType + Any + Sized {
    type Machine: Machine<
            Recipe: Recipe<Outputs = (Resource<Self>,)>
                        + ConstRecipe<BundledInputs: Makeable, BundledOutputs = (Bundle<Self, 1>,)>,
        > + Makeable;
}
impl<R> BundleMakeable for R
where
    R: MachineMakeable,
{
    fn craft_one(state: &mut GameState) -> WakeHandle<Bundle<Self, 1>> {
        state.make_then(|state, inputs| state.craft::<R::Machine, _>(inputs))
    }
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
impl MachineMakeable for ElectronicCircuit {
    type Machine = Assembler<ElectronicCircuitRecipe>;
}
impl MachineMakeable for Point {
    type Machine = Assembler<PointRecipe>;
}
impl<T: Technology + Any> MachineMakeable for ResearchPoint<T>
where
    Lab<T>: Makeable,
    TechRecipe<T>: Recipe<Inputs: MachineInputs, Outputs = (Resource<Self>,)>
        + ConstRecipe<BundledInputs: Makeable, BundledOutputs = (Bundle<Self, 1>,)>,
{
    type Machine = Lab<T>;
}

trait ConstMakeable {
    const MAKE: Self;
}
impl<T: ConstMakeable + Any> Makeable for T {
    fn make(state: &mut GameState) -> WakeHandle<Self> {
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
    pub fn make<T: Makeable>(&mut self) -> WakeHandle<T> {
        T::make(self)
    }

    /// Schedules `f` to run after `h` completes, and returns a hendl to the final output.
    pub fn make_then<T: Makeable, U: Any>(
        &mut self,
        f: impl FnOnce(&mut GameState, T) -> WakeHandle<U> + 'static,
    ) -> WakeHandle<U> {
        let x = self.make::<T>();
        self.then(x, f)
    }

    /// Given a producer of a single bundle of an item, make a producer of a larger bundle.
    pub fn multiple<const SINGLE: u32, const COUNT: u32, R: ResourceType + Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> WakeHandle<Bundle<R, SINGLE>>,
    ) -> WakeHandle<Bundle<R, COUNT>> {
        assert_eq!(COUNT.rem_euclid(SINGLE), 0);
        let singles = (0..COUNT / SINGLE).map(|_| f(self)).collect_vec();
        let sum = self.collect_sum(singles);
        self.map(sum, |_, mut sum| sum.bundle().unwrap())
    }

    /// Waits until the given resource has the required amount then returns that amount of
    /// resource.
    pub fn wait_for_resource<const AMOUNT: u32, R: ResourceType + Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> &mut Resource<R> + 'static,
    ) -> WakeHandle<Bundle<R, AMOUNT>> {
        self.wait_for(move |state| f(state).bundle().ok())
    }

    /// Waits until the selected machine has produced enough output.
    /// This is the main wait point of our system.
    fn wait_for_machine_output<M>(
        &mut self,
        machine_id: MachineSlot<M>,
    ) -> WakeHandle<<M::Recipe as ConstRecipe>::BundledOutputs>
    where
        M: Machine + Makeable,
        M::Recipe: ConstRecipe + Any,
    {
        self.wait_for(move |state| {
            let machine = state.resources.machine_store.get(machine_id);
            M::Recipe::get_outputs(&mut machine.outputs(&state.tick))
        })
    }

    /// Craft an item using the provided machine.
    pub fn craft<M, O>(
        &mut self,
        inputs: <M::Recipe as ConstRecipe>::BundledInputs,
    ) -> WakeHandle<O>
    where
        M: Machine + Makeable,
        M::Recipe: ConstRecipe<BundledOutputs = (O,)> + Any,
        O: Any,
    {
        self.make_then(move |state, slot: MachineSlot<M>| {
            let machine = state.resources.machine_store.get(slot);
            let machine_inputs = machine.inputs(&state.tick);
            M::Recipe::add_inputs(machine_inputs, inputs);
            let out = state.wait_for_machine_output(slot);
            state.map(out, |_, out| out.0)
        })
    }

    fn hand_mine<Ore: ResourceType + Any>(
        &mut self,
        territory: fn(&mut Resources) -> &mut Territory<Ore>,
    ) -> WakeHandle<Bundle<Ore, 1>> {
        self.with_mut_tick(move |state, mut_token| {
            let t = territory(&mut state.resources);
            let mut_tick = state.tick.as_mut(mut_token);
            let out: Bundle<Ore, 1> = t.hand_mine(mut_tick);
            t.resources(&state.tick).add(out);
        });
        self.wait_for_resource(move |state| territory(&mut state.resources).resources(&state.tick))
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
}
