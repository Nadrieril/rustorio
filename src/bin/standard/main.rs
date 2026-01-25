#![forbid(unsafe_code)]
use std::any::Any;

use itertools::Itertools;
use rustorio::{
    self, Bundle, HandRecipe, Recipe, ResearchPoint, Resource, ResourceType, Technology, Tick,
    buildings::{Assembler, Furnace, Lab},
    gamemodes::Standard,
    recipes::{
        AssemblerRecipe, CopperSmelting, CopperWireRecipe, ElectronicCircuitRecipe, FurnaceRecipe,
        IronSmelting, RedScienceRecipe,
    },
    research::{RedScience, SteelTechnology},
    resources::{Copper, CopperOre, CopperWire, ElectronicCircuit, Iron, IronOre, Point},
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

struct GameState {
    tick: Tick,
    resources: Resources,
    queue: WaiterQueue,
}

struct Resources {
    iron_territory: Territory<IronOre>,
    copper_territory: Territory<CopperOre>,
    steel_technology: SteelTechnology,

    iron: Resource<Iron>,

    iron_furnace: Option<Furnace<IronSmelting>>,
    copper_furnace: Option<Furnace<CopperSmelting>>,
    copper_wire_assembler: Option<Assembler<CopperWireRecipe>>,
    elec_circuit_assembler: Option<Assembler<ElectronicCircuitRecipe>>,
    steel_lab: Option<Lab<SteelTechnology>>,
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
        GameState {
            tick,
            queue: Default::default(),
            resources: Resources {
                iron_territory,
                copper_territory,
                steel_technology,

                iron: iron.to_resource(),
                iron_furnace: Default::default(),
                copper_furnace: Default::default(),
                copper_wire_assembler: Default::default(),
                elec_circuit_assembler: Default::default(),
                steel_lab: Default::default(),
            },
        }
    }

    fn tick(&mut self) {
        self.tick.advance();
        self.check_waiters();
    }
    #[expect(unused)]
    fn advance_by(&mut self, t: u64) {
        for _ in 0..t {
            self.tick()
        }
        println!("{}", self.tick);
    }
    fn complete<R: Any>(&mut self, h: WakeHandle<R>) -> R {
        loop {
            if let Some(ret) = self.queue.get(h) {
                println!("{}", self.tick);
                return ret;
            }
            self.tick();
        }
    }
}

trait Machine<R: Recipe> {
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs;
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs;
}

impl<R: FurnaceRecipe> Machine<R> for Furnace<R> {
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<R: AssemblerRecipe> Machine<R> for Assembler<R> {
    fn inputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <R as Recipe>::Outputs {
        self.outputs(tick)
    }
}
impl<T: Technology> Machine<TechRecipe<T>> for Lab<T> {
    fn inputs(&mut self, tick: &Tick) -> &mut <TechRecipe<T> as Recipe>::Inputs {
        self.inputs(tick)
    }
    fn outputs(&mut self, tick: &Tick) -> &mut <TechRecipe<T> as Recipe>::Outputs {
        self.outputs(tick)
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

    /// Waits until the function returns `Some` and yields the returned value.
    fn wait_for<T: Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> Option<T> + 'static,
    ) -> WakeHandle<T> {
        enum W<F, T> {
            Waiting(F),
            Ready(T),
        }
        impl<F, T: Any> Waiter for W<F, T>
        where
            F: Fn(&mut GameState) -> Option<T>,
        {
            type Output = T;
            fn is_ready(&mut self, state: &mut GameState) -> bool {
                match self {
                    W::Waiting(f) => match f(state) {
                        Some(x) => {
                            *self = W::Ready(x);
                            true
                        }
                        None => false,
                    },
                    W::Ready(_) => true,
                }
            }
            fn wake(self, _state: &mut GameState) -> T {
                let W::Ready(x) = self else { unreachable!() };
                x
            }
        }
        self.enqueue_waiter(W::Waiting(f))
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
        f: fn(&mut GameState) -> WakeHandle<Bundle<R, SINGLE>>,
    ) -> WakeHandle<Bundle<R, COUNT>> {
        assert_eq!(COUNT.rem_euclid(SINGLE), 0);
        let singles = (0..COUNT / SINGLE).map(|_| f(self)).collect_vec();
        let sum = self.collect_sum(singles);
        self.map(sum, |_, mut sum| sum.bundle().unwrap())
    }

    /// Craft an item using the provided machine.
    fn craft<const ICOUNT: u32, const OCOUNT: u32, R, M, I, O>(
        &mut self,
        input: Bundle<I, ICOUNT>,
        f: fn(&mut Resources) -> &mut Option<M>,
    ) -> WakeHandle<Bundle<O, OCOUNT>>
    where
        R: Recipe<Inputs = (Resource<I>,), InputAmountsType = (u32,)>,
        R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
        M: Machine<R> + Any,
        I: ResourceType + Any,
        O: ResourceType + Any,
    {
        // Check proportionality of the recipe.
        assert_eq!(ICOUNT * R::OUTPUT_AMOUNTS.0, OCOUNT * R::INPUT_AMOUNTS.0);
        let machine_ready = self.wait_until(move |s| f(&mut s.resources).is_some());
        self.then(machine_ready, move |state, _| {
            let machine = f(&mut state.resources).as_mut().unwrap();
            let inputs = machine.inputs(&state.tick);
            inputs.0.add(input);
            state.wait_for_resource(move |state| {
                let machine = f(&mut state.resources).as_mut().unwrap();
                &mut machine.outputs(&state.tick).0
            })
        })
    }

    /// Craft an item using the provided machine.
    fn craft2<const I1COUNT: u32, const I2COUNT: u32, const OCOUNT: u32, R, M, I1, I2, O>(
        &mut self,
        input1: Bundle<I1, I1COUNT>,
        input2: Bundle<I2, I2COUNT>,
        f: fn(&mut Resources) -> &mut Option<M>,
    ) -> WakeHandle<Bundle<O, OCOUNT>>
    where
        R: Recipe<Inputs = (Resource<I1>, Resource<I2>), InputAmountsType = (u32, u32)>,
        R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
        M: Machine<R> + Any,
        I1: ResourceType + Any,
        I2: ResourceType + Any,
        O: ResourceType + Any,
    {
        // Check proportionality of the recipe.
        assert_eq!(I1COUNT * R::OUTPUT_AMOUNTS.0, OCOUNT * R::INPUT_AMOUNTS.0);
        assert_eq!(I2COUNT * R::OUTPUT_AMOUNTS.0, OCOUNT * R::INPUT_AMOUNTS.1);
        let machine_ready = self.wait_until(move |s| f(&mut s.resources).is_some());
        self.then(machine_ready, move |state, _| {
            let machine = f(&mut state.resources).as_mut().unwrap();
            let inputs = machine.inputs(&state.tick);
            inputs.0.add(input1);
            inputs.1.add(input2);
            state.wait_for_resource(move |state| {
                let machine = f(&mut state.resources).as_mut().unwrap();
                &mut machine.outputs(&state.tick).0
            })
        })
    }
}

impl GameState {
    fn iron_ore(&mut self) -> WakeHandle<Bundle<IronOre, 1>> {
        if self.resources.iron_territory.num_miners() == 0 {
            let ore = self.resources.iron_territory.hand_mine(&mut self.tick);
            self.nowait(ore)
        } else {
            self.wait_for_resource(|state| state.resources.iron_territory.resources(&state.tick))
        }
    }
    fn copper_ore(&mut self) -> WakeHandle<Bundle<CopperOre, 1>> {
        if self.resources.copper_territory.num_miners() == 0 {
            let ore = self.resources.copper_territory.hand_mine(&mut self.tick);
            self.nowait(ore)
        } else {
            self.wait_for_resource(|state| state.resources.copper_territory.resources(&state.tick))
        }
    }

    fn iron<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Iron, COUNT>> {
        self.multiple::<1, COUNT, _>(|state| {
            if let Ok(x) = state.resources.iron.bundle() {
                return state.nowait(x);
            } else {
                let h = state.iron_ore();
                state.then(h, |state, ore| {
                    state.craft(ore, |resources| &mut resources.iron_furnace)
                })
            }
        })
    }

    fn copper<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Copper, COUNT>> {
        self.multiple::<1, COUNT, _>(|state| {
            let h = state.copper_ore();
            state.then(h, |state, ore| {
                state.craft(ore, |resources| &mut resources.copper_furnace)
            })
        })
    }

    fn furnace<R: FurnaceRecipe + Any>(&mut self, r: R) -> WakeHandle<Furnace<R>> {
        let iron = self.iron();
        self.map(iron, |state, iron| Furnace::build(&state.tick, r, iron))
    }
    fn add_furnace<R: FurnaceRecipe + Any>(
        &mut self,
        r: R,
        f: fn(&mut Resources) -> &mut Option<Furnace<R>>,
    ) -> WakeHandle<()> {
        let furnace = self.furnace(r);
        self.map(furnace, move |state, furnace| {
            *f(&mut state.resources) = Some(furnace);
        })
    }

    fn miner(&mut self) -> WakeHandle<Miner> {
        let iron = self.iron();
        let copper = self.copper();
        let both = self.pair(iron, copper);
        self.map(both, |_state, (iron, copper)| Miner::build(iron, copper))
    }
    fn add_miner<R: ResourceType + Any>(
        &mut self,
        f: fn(&mut Resources) -> &mut Territory<R>,
    ) -> WakeHandle<()> {
        let miner = self.miner();
        self.map(miner, move |state, miner| {
            f(&mut state.resources)
                .add_miner(&state.tick, miner)
                .unwrap();
        })
    }

    fn copper_wire<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<CopperWire, COUNT>> {
        self.multiple(|state| {
            let copper = state.copper::<1>();
            state.then(copper, |state, copper| {
                if state.resources.copper_wire_assembler.is_some() {
                    state.craft(copper, |resources| &mut resources.copper_wire_assembler)
                } else {
                    let out = CopperWireRecipe::craft(&mut state.tick, (copper,)).0;
                    state.nowait(out)
                }
            })
        })
    }

    fn assembler<R: AssemblerRecipe + Any>(&mut self, r: R) -> WakeHandle<Assembler<R>> {
        let iron = self.iron();
        let copper_wire = self.copper_wire();
        let both = self.pair(copper_wire, iron);
        self.map(both, |state, (copper_wire, iron)| {
            Assembler::build(&state.tick, r, copper_wire, iron)
        })
    }
    fn add_assembler<R: AssemblerRecipe + Any>(
        &mut self,
        r: R,
        f: fn(&mut Resources) -> &mut Option<Assembler<R>>,
    ) -> WakeHandle<()> {
        let assembler = self.assembler(r);
        self.map(assembler, move |state, assembler| {
            *f(&mut state.resources) = Some(assembler);
        })
    }

    fn lab<T: Technology + Any>(&mut self, get_tech: fn(&Resources) -> &T) -> WakeHandle<Lab<T>> {
        let iron = self.iron();
        let copper = self.copper();
        let both = self.pair(iron, copper);
        self.map(both, move |state, (iron, copper)| {
            let tech = get_tech(&state.resources);
            Lab::build(&state.tick, tech, iron, copper)
        })
    }
    fn add_lab<T: Technology + Any>(
        &mut self,
        get_tech: fn(&Resources) -> &T,
        f: fn(&mut Resources) -> &mut Option<Lab<T>>,
    ) -> WakeHandle<()> {
        let lab = self.lab(get_tech);
        self.map(lab, move |state, lab| {
            *f(&mut state.resources) = Some(lab);
        })
    }

    fn circuit<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<ElectronicCircuit, COUNT>> {
        self.multiple::<1, COUNT, _>(|state| {
            let iron = state.iron::<1>();
            let copper_wire = state.copper_wire::<2>();
            let both = state.pair(copper_wire, iron);
            state.then(both, |state, (copper_wire, iron)| {
                state.craft2(iron, copper_wire, |r| &mut r.elec_circuit_assembler)
            })
        })
    }

    fn red_science<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<RedScience, COUNT>> {
        self.multiple::<1, COUNT, _>(|state| {
            let iron = state.iron();
            let circuit = state.circuit();
            let both = state.pair(iron, circuit);
            state.map(both, |state, (copper_wire, iron)| {
                RedScienceRecipe::craft(&mut state.tick, (copper_wire, iron)).0
            })
        })
    }

    fn play(mut self) -> (Tick, Bundle<Point, 200>) {
        let h = self.add_furnace(IronSmelting, |r| &mut r.iron_furnace);
        self.complete(h);

        let h = self.add_furnace(CopperSmelting, |r| &mut r.copper_furnace);
        self.complete(h);

        let _ = self.add_miner(|r| &mut r.iron_territory);
        let _ = self.add_miner(|r| &mut r.copper_territory);

        let h = self.add_assembler(CopperWireRecipe, |r| &mut r.copper_wire_assembler);
        self.complete(h);

        let h = self.add_assembler(ElectronicCircuitRecipe, |r| &mut r.elec_circuit_assembler);
        self.complete(h);

        let h = self.add_lab(|r| &r.steel_technology, |r| &mut r.steel_lab);
        self.complete(h);

        let science = self.red_science::<1>();
        let science: Bundle<RedScience, 1> = self.complete(science);

        let h = self.craft(science, |r| &mut r.steel_lab);
        let _research: Bundle<ResearchPoint<SteelTechnology>, 1> = self.complete(h);

        todo!("WIP: {}", self.tick.cur())
    }
}
