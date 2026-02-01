use std::any::Any;

use crate::*;

pub trait SingleOutputMachine:
    Machine<Recipe: ConstRecipe<BundledOutputs = (Self::Output,)>>
{
    type Output;
}
impl<M: Machine<Recipe: ConstRecipe<BundledOutputs = (O,)>>, O> SingleOutputMachine for M {
    type Output = O;
}

/// Set of resources that can be automatically crafted.
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

impl<const AMOUNT: u32, R: BundleMakeable> Makeable for Bundle<R, AMOUNT>
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
        <R as BundleMakeable>::add_node_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        <R as BundleMakeable>::add_edge_to_graph(graph, start, weight * AMOUNT as f32);
    }
}

pub use bundle_makeable::*;
mod bundle_makeable {
    use crate::*;

    pub trait SingleOutputProducer:
        Producer<Output = (<Self as SingleOutputProducer>::Output,)>
    {
        type Output;
    }
    impl<P: Producer<Output = (O,)>, O> SingleOutputProducer for P {
        type Output = O;
    }

    /// Items that can be produced by whole bundle amounts. This is the heart of the crafting logic.
    /// This maker fetches the required inputs, gives them to the producer, then waits for the producer
    /// to produce its output.
    pub trait BundleMakeable: ResourceType + Sized + Any {
        type Producer: SingleOutputProducer<Input: Makeable, Output: IsBundle<Resource = Self>>;

        /// Record this resource in the global resource graph.
        fn add_node_to_graph(graph: &mut ResourceGraph) {
            if let Some(id) = graph.add_node::<Self>() {
                let weight = 1f32 / <Self::Producer as SingleOutputProducer>::Output::AMOUNT as f32;
                <<Self::Producer as Producer>::Input as Makeable>::add_edge_to_graph(
                    graph, id, weight,
                );
            }
        }
        /// Record an edge to this resource in the global resource graph.
        fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
            Self::add_node_to_graph(graph);
            graph.add_edge_to::<Self>(start, weight);
        }
    }

    impl BundleMakeable for IronOre {
        type Producer = Territory<Self>;
    }
    impl BundleMakeable for CopperOre {
        type Producer = Territory<Self>;
    }
    impl BundleMakeable for RedScience {
        type Producer = HandCrafter<RedScienceRecipe>;
    }
    impl<R: MachineMakeable> BundleMakeable for R {
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
}

impl<R: SingleMakeable> Makeable for R {
    fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
        <Self as SingleMakeable>::make_to(state, p, sink)
    }

    fn add_nodes_to_graph(graph: &mut ResourceGraph) {
        <Self as SingleMakeable>::add_node_to_graph(graph);
    }
    fn add_edge_to_graph(graph: &mut ResourceGraph, start: GraphNode, weight: f32) {
        <Self as SingleMakeable>::add_edge_to_graph(graph, start, weight);
    }
}

pub use single_makeable::*;
mod single_makeable {
    use crate::*;

    /// Items that can be automatically crafted.
    pub trait SingleMakeable: Sized + Any {
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

    impl SingleMakeable for Miner {
        type Input = (Bundle<Iron, 10>, Bundle<Copper, 5>);

        fn make_from_input(_state: &mut GameState, (iron, copper): Self::Input) -> Self {
            Miner::build(iron, copper)
        }
    }
    impl<R> SingleMakeable for Furnace<R>
    where
        R: FurnaceRecipe + Recipe + Copy,
        Available<R>: Makeable,
    {
        type Input = (Available<R>, Bundle<Iron, 10>);

        fn make_from_input(state: &mut GameState, (r, iron): Self::Input) -> Self {
            let r = *state.resources.reusable().get(r);
            Furnace::build(&state.tick, r, iron)
        }
    }
    impl<R> SingleMakeable for Assembler<R>
    where
        R: AssemblerRecipe + Recipe + Copy,
        Available<R>: Makeable,
    {
        type Input = (Available<R>, Bundle<Iron, 6>, Bundle<CopperWire, 12>);

        fn make_from_input(state: &mut GameState, (r, iron, copper_wire): Self::Input) -> Self {
            let r = *state.resources.reusable().get(r);
            Assembler::build(&state.tick, r, copper_wire, iron)
        }
    }
    impl<T: Technology> SingleMakeable for Lab<T>
    where
        Available<T>: Makeable,
    {
        type Input = (Available<T>, Bundle<Iron, 20>, Bundle<Copper, 15>);

        fn make_from_input(state: &mut GameState, (tech, iron, copper): Self::Input) -> Self {
            let tech = state.resources.reusable().get(tech);
            Lab::build(&state.tick, tech, iron, copper)
        }
    }

    impl SingleMakeable for Available<SteelTechnology> {
        type Input = ();
        fn make_from_input(state: &mut GameState, _input: Self::Input) -> Self {
            state.resources.reusable().available().unwrap()
        }
    }
    impl SingleMakeable for Available<PointsTechnology> {
        // The steel smelting recipe is because it also sets up the points tech.
        type Input = Available<SteelSmelting>;
        fn make_from_input(state: &mut GameState, _input: Self::Input) -> Self {
            state.resources.reusable().available().unwrap()
        }
    }

    impl SingleMakeable for TheFirstTime<SteelSmelting> {
        type Input = (
            Available<SteelTechnology>,
            Bundle<ResearchPoint<SteelTechnology>, 20>,
        );

        fn make_from_input(
            state: &mut GameState,
            (steel_tech, research_points): Self::Input,
        ) -> Self {
            let steel_tech: SteelTechnology = state.resources.reusable().take(steel_tech);
            let (steel_smelting, points_tech) = steel_tech.research(research_points);
            let pqw = state.resources.machine::<Lab<SteelTechnology>>();
            assert_eq!(pqw.queue.len(), 0);
            let lab = pqw
                .producer
                .take_map(|lab| lab.change_technology(&points_tech).unwrap());
            println!("changing the labs to `PointsTechnology`");
            *state.resources.machine() = ProducerWithQueue::new(lab);
            state.resources.reusable().set(points_tech);
            TheFirstTime(steel_smelting)
        }
    }
    impl SingleMakeable for Available<SteelSmelting> {
        type Input = TheFirstTime<SteelSmelting>;

        fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
            // If we let scaling up happen automatically, we apparently lose ticks :(
            state.scale_up::<OnceMaker<SteelSmelting>>(p);
            state.produce_to_state_sink::<OnceMaker<SteelSmelting>>(p, sink);
        }

        fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
            unreachable!()
        }
    }

    impl SingleMakeable for TheFirstTime<PointRecipe> {
        type Input = (
            Available<PointsTechnology>,
            Bundle<ResearchPoint<PointsTechnology>, 50>,
        );

        fn make_from_input(
            state: &mut GameState,
            (points_tech, research_points): Self::Input,
        ) -> Self {
            let points_tech: PointsTechnology = state.resources.reusable().take(points_tech);
            let points_recipe = points_tech.research(research_points);
            TheFirstTime(points_recipe)
        }
    }
    impl SingleMakeable for Available<PointRecipe> {
        type Input = TheFirstTime<PointRecipe>;

        fn make_to(state: &mut GameState, p: Priority, sink: StateSink<Self>) {
            // If we let scaling up happen automatically, we apparently lose ticks :(
            state.scale_up::<OnceMaker<PointRecipe>>(p);
            state.produce_to_state_sink::<OnceMaker<PointRecipe>>(p, sink);
        }

        fn make_from_input(_state: &mut GameState, _input: Self::Input) -> Self {
            unreachable!()
        }
    }

    impl<T: BaseRecipe + Any> SingleMakeable for Available<T> {
        type Input = ();
        fn make_from_input(state: &mut GameState, _: ()) -> Self {
            state.resources.reusable().set(T::MAKE)
        }
    }

    pub use const_makeable::BaseRecipe;
    mod const_makeable {
        use crate::*;

        pub trait BaseRecipe: Recipe + Copy {
            const MAKE: Self;
        }

        impl BaseRecipe for IronSmelting {
            const MAKE: Self = IronSmelting;
        }
        impl BaseRecipe for CopperSmelting {
            const MAKE: Self = CopperSmelting;
        }
        impl BaseRecipe for CopperWireRecipe {
            const MAKE: Self = CopperWireRecipe;
        }
        impl BaseRecipe for ElectronicCircuitRecipe {
            const MAKE: Self = ElectronicCircuitRecipe;
        }
    }
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
