use std::{any::Any, marker::PhantomData};

use crate::*;

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

/// Set of resources that can be automatically crated.
pub trait Makeable: Any + Sized {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>);

    /// Record these resources in the global resource graph.
    fn add_nodes_to_graph(graph: &mut ResourceGraph);
    /// Record an edge to these resources in the global resource graph.
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32);
}
impl Makeable for () {
    fn make_to(state: &mut GameState, _p: Priority, sink: StateSink<Self>) {
        sink.give(state, ());
    }

    fn add_nodes_to_graph(_graph: &mut ResourceGraph) {}
    fn add_edge_to_graph(_graph: &mut ResourceGraph, _start: GraphNode, _weight: f32) {}
}
impl<A: Makeable> Makeable for (A,) {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        A::make_to(state, p, sink.map(|_, v| (v,)))
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        A::add_nodes_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        A::add_edge_to_graph(graph, start, weight);
    }
}
impl<A: Makeable, B: Makeable> Makeable for (A, B) {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        let (a, b) = sink.split();
        A::make_to(state, p, a);
        B::make_to(state, p, b);
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        A::add_nodes_to_graph(graph);
        B::add_nodes_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        A::add_edge_to_graph(graph, start, weight);
        B::add_edge_to_graph(graph, start, weight);
    }
}
impl<A: Makeable, B: Makeable, C: Makeable> Makeable for (A, B, C) {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        let sink = sink.map(|_, ((x, y), z)| (x, y, z));
        let (ab, c) = sink.split();
        let (a, b) = ab.split();
        A::make_to(state, p, a);
        B::make_to(state, p, b);
        C::make_to(state, p, c);
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        A::add_nodes_to_graph(graph);
        B::add_nodes_to_graph(graph);
        C::add_nodes_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        A::add_edge_to_graph(graph, start, weight);
        B::add_edge_to_graph(graph, start, weight);
        C::add_edge_to_graph(graph, start, weight);
    }
}
impl<const N: usize, T: Makeable> Makeable for [T; N] {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        for sink in sink.split_n() {
            T::make_to(state, p, sink);
        }
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        T::add_nodes_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        T::add_edge_to_graph(graph, start, weight * N as f32);
    }
}
impl<R: InputMakeable> Makeable for R {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        <Self as InputMakeable>::make_to(state, p, sink)
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        <Self as InputMakeable>::add_node_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        <Self as InputMakeable>::add_edge_to_graph(graph, start, weight);
    }
}

/// Items that can be automatically crafted.
pub trait InputMakeable: Sized + Any {
    type Input: Makeable;

    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        let sink = sink.map(Self::make_from_input);
        // TODO: Weirdly enough, without the stateless indirection I get worse ticks due to the
        // different (earlier?) scheduling.
        let sink = state.make_stateless(sink).with_gamestate();
        state.make_to(p, sink);
    }

    fn make_from_input(state: &mut GameState, input: Self::Input) -> Self;

    /// Record this resource in the global resource graph.
    fn add_node_to_graph(graph: &mut ResourceGraph) {
        if let Some(id) = graph.add_node::<Self>() {
            Self::Input::add_edge_to_graph(graph, id, 1f32);
        }
    }
    /// Record an edge to this resource in the global resource graph.
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        Self::add_node_to_graph(graph);
        graph.add_edge_to::<Self>(start, weight);
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
        println!("changing the labs to `PointsTechnology`");
        *state.resources.machine() = ProducerWithQueue::new(lab);
        *state.resources.tech() = Some(points_tech);
        TheFirstTime(steel_smelting)
    }
}
impl InputMakeable for SteelSmelting {
    type Input = ();

    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        // If we let scaling up happen automatically, we apparently lose ticks :(
        state.scale_up::<OnceMaker<Self>>(p);
        state.produce_to_state_sink::<OnceMaker<Self>>(p, sink);
    }

    fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
        unreachable!()
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
impl InputMakeable for PointRecipe {
    type Input = ();

    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        // If we let scaling up happen automatically, we apparently lose ticks :(
        state.scale_up::<OnceMaker<Self>>(p);
        state.produce_to_state_sink::<OnceMaker<Self>>(p, sink);
    }

    fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
        unreachable!()
    }
}

impl<const AMOUNT: u32, R: ProducerMakeable> Makeable for Bundle<R, AMOUNT>
where
    [(); (AMOUNT / <R::Producer as SingleOutputProducer>::Output::AMOUNT) as usize]:,
{
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        if let Ok(x) = state.resources.resource().bundle() {
            sink.give(state, x);
            return;
        }
        // We cleverly don't fetch the whole input at once. Instead, as soon as the first input
        // bundle arrives we feed it to the producer.
        // Split the sink into individual chunks that match what the produces produces.
        for sink in sink.split_resource() {
            let sink = sink.map::<<R::Producer as Producer>::Output>(|_, out| out.0);
            state.produce_to_state_sink::<R::Producer>(p, sink);
        }
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        <R as ProducerMakeable>::add_node_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        <R as ProducerMakeable>::add_edge_to_graph(graph, start, weight * AMOUNT as f32);
    }
}

/// Items that can be produced by producers. This is the heart of the crafting logic.
/// This maker fetches the required inputs, gives them to the producer, then waits for the producer
/// to produce its output.
pub trait ProducerMakeable: ResourceType + Sized + Any {
    type Producer: SingleOutputProducer<Input: Makeable, Output: IsBundle<Resource = Self>>;

    /// Record this resource in the global resource graph.
    fn add_node_to_graph(graph: &mut ResourceGraph) {
        if let Some(id) = graph.add_node::<Self>() {
            let weight = 1f32 / <Self::Producer as SingleOutputProducer>::Output::AMOUNT as f32;
            <<Self::Producer as Producer>::Input as Makeable>::add_edge_to_graph(graph, id, weight);
        }
    }
    /// Record an edge to this resource in the global resource graph.
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        Self::add_node_to_graph(graph);
        graph.add_edge_to::<Self>(start, weight);
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

pub trait ConstMakeable {
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

impl GameState {
    pub fn make<T: Makeable>(&mut self, p: Priority) -> WakeHandle<T> {
        let (h, sink) = WakeHandle::make_pipe();
        self.make_to(p, sink);
        h
    }
    pub fn make_to<T: Makeable>(&mut self, p: Priority, sink: StateSink<T>) {
        T::add_nodes_to_graph(&mut self.graph);
        T::make_to(self, p, sink)
    }

    /// Give input to the producer and wait for it to produce output.
    /// This is the main wait point of our system.
    pub fn produce_to_sink<P: Producer<Input: Makeable>>(
        &mut self,
        p: Priority,
        sink: Sink<P::Output>,
    ) {
        self.make_to(p, P::feed(p, sink));
    }
    pub fn produce_to_state_sink<P: Producer<Input: Makeable>>(
        &mut self,
        p: Priority,
        sink: StateSink<P::Output>,
    ) {
        // Does the conversion between sink types via the `CallBackQueue`.
        // TODO: Can't use `make_stateless` because that subtly changes the scheduling and
        // regresses our time.
        self.handle_via_sink::<<P as Producer>::Output>(|state, sink| {
            state.produce_to_sink::<P>(p, sink);
        })
        .set_sink(self, sink);
    }
}
