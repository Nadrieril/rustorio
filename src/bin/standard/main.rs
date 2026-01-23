#![forbid(unsafe_code)]
use std::any::Any;

use itertools::Itertools;
use rustorio::{
    self, Bundle, HandRecipe, Recipe, Resource, ResourceType, Tick,
    buildings::{Assembler, Furnace},
    gamemodes::Standard,
    recipes::{CopperSmelting, CopperWireRecipe, FurnaceRecipe, IronSmelting},
    research::SteelTechnology,
    resources::{Copper, CopperOre, Iron, IronOre, Point},
    territory::{Miner, Territory},
};

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

    iron_territory: Territory<IronOre>,
    copper_territory: Territory<CopperOre>,
    #[expect(unused)]
    steel_technology: SteelTechnology,

    iron: Resource<Iron>,
    #[expect(unused)]
    copper: Resource<Copper>,

    hand_iron_furnace: Option<Furnace<IronSmelting>>,
    hand_copper_furnace: Option<Furnace<CopperSmelting>>,

    queue: WaiterQueue,
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

            iron_territory,
            copper_territory,
            steel_technology,

            iron: iron.to_resource(),
            copper: Resource::new_empty(),
            hand_iron_furnace: Default::default(),
            hand_copper_furnace: Default::default(),

            queue: Default::default(),
        }
    }

    fn tick(&mut self) {
        self.tick.advance();
        self.check_waiters();
    }
    fn advance_by(&mut self, t: u64) {
        for _ in 0..t {
            self.tick()
        }
        println!("{}", self.tick);
    }
    fn wait_for<R: Any>(&mut self, h: WakeHandle<R>) -> R {
        loop {
            if let Some(ret) = self.queue.get(h) {
                println!("{}", self.tick);
                break ret;
            }
            self.tick();
        }
    }

    /// Waits until the given resource has the required amount then returns that amount of
    /// resource.
    fn get_resource<const AMOUNT: u32, R: ResourceType + Any>(
        &mut self,
        f: impl Fn(&mut GameState) -> &mut Resource<R> + 'static,
    ) -> WakeHandle<Bundle<R, AMOUNT>> {
        struct W<const AMOUNT: u32, F>(F);
        impl<const AMOUNT: u32, F, R: ResourceType + Any> Waiter for W<AMOUNT, F>
        where
            F: Fn(&mut GameState) -> &mut Resource<R>,
        {
            type Output = Bundle<R, AMOUNT>;
            fn is_ready(&mut self, state: &mut GameState) -> bool {
                (self.0)(state).amount() >= AMOUNT
            }
            fn wake(self, state: &mut GameState) -> Bundle<R, AMOUNT> {
                (self.0)(state).bundle().unwrap()
            }
        }
        self.enqueue_waiter(W(f))
    }

    fn get_iron_ore<const AMOUNT: u32>(&mut self) -> WakeHandle<Bundle<IronOre, AMOUNT>> {
        if self.iron_territory.num_miners() == 0 {
            let ore = self.iron_territory.hand_mine(&mut self.tick);
            self.queue.set_already_resolved_handle(ore)
        } else {
            self.get_resource::<AMOUNT, _>(|state| state.iron_territory.resources(&state.tick))
        }
    }
    fn get_copper_ore<const AMOUNT: u32>(&mut self) -> WakeHandle<Bundle<CopperOre, AMOUNT>> {
        if self.copper_territory.num_miners() == 0 {
            let ore = self.copper_territory.hand_mine(&mut self.tick);
            self.queue.set_already_resolved_handle(ore)
        } else {
            self.get_resource::<AMOUNT, _>(|state| state.copper_territory.resources(&state.tick))
        }
    }

    fn smelt_iron<const COUNT: u32>(
        &mut self,
        ore: Bundle<IronOre, COUNT>,
    ) -> WakeHandle<Bundle<Iron, COUNT>> {
        self.hand_iron_furnace
            .as_mut()
            .unwrap()
            .inputs(&self.tick)
            .0
            .add(ore);
        self.get_resource(|state| {
            &mut state
                .hand_iron_furnace
                .as_mut()
                .unwrap()
                .outputs(&state.tick)
                .0
        })
    }
    fn smelt_copper<const COUNT: u32>(
        &mut self,
        ore: Bundle<CopperOre, COUNT>,
    ) -> WakeHandle<Bundle<Copper, COUNT>> {
        self.hand_copper_furnace
            .as_mut()
            .unwrap()
            .inputs(&self.tick)
            .0
            .add(ore);
        self.get_resource(|state| {
            &mut state
                .hand_copper_furnace
                .as_mut()
                .unwrap()
                .outputs(&state.tick)
                .0
        })
    }

    fn mine_and_smelt_iron<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Iron, COUNT>> {
        match self.iron.bundle() {
            Ok(x) => return self.queue.set_already_resolved_handle(x),
            Err(_) => {}
        }
        let h = self.get_iron_ore();
        self.then(h, |state, ore| state.smelt_iron(ore))
    }
    fn mine_and_smelt_copper<const COUNT: u32>(&mut self) -> WakeHandle<Bundle<Copper, COUNT>> {
        let h = self.get_copper_ore();
        self.then(h, |state, ore| state.smelt_copper(ore))
    }

    fn make_furnace<R: FurnaceRecipe + Any>(&mut self, r: R) -> WakeHandle<Furnace<R>> {
        let iron = self.mine_and_smelt_iron();
        self.then(iron, |state, iron| {
            let furnace = Furnace::build(&state.tick, r, iron);
            state.queue.set_already_resolved_handle(furnace)
        })
    }

    fn make_miner(&mut self) -> WakeHandle<Miner> {
        let iron = self.mine_and_smelt_iron();
        let copper = self.mine_and_smelt_copper();
        let both = self.pair(iron, copper);
        self.then(both, |state, (iron, copper)| {
            let miner = Miner::build(iron, copper);
            state.queue.set_already_resolved_handle(miner)
        })
    }
    fn add_iron_miner(&mut self) -> WakeHandle<()> {
        let miner = self.make_miner();
        self.then(miner, |state, miner| {
            state.iron_territory.add_miner(&state.tick, miner).unwrap();
            state.queue.set_already_resolved_handle(())
        })
    }
    fn add_copper_miner(&mut self) -> WakeHandle<()> {
        let miner = self.make_miner();
        self.then(miner, |state, miner| {
            state
                .copper_territory
                .add_miner(&state.tick, miner)
                .unwrap();
            state.queue.set_already_resolved_handle(())
        })
    }

    fn play(mut self) -> (Tick, Bundle<Point, 200>) {
        let furnace = self.make_furnace(IronSmelting);
        let furnace = self.wait_for(furnace);
        self.hand_iron_furnace = Some(furnace);

        let furnace = self.make_furnace(CopperSmelting);
        let furnace = self.wait_for(furnace);
        self.hand_copper_furnace = Some(furnace);

        self.add_iron_miner();
        self.add_copper_miner();

        let iron = self.mine_and_smelt_iron();
        let wire_bundles = (0..6)
            .into_iter()
            .map(|_| {
                let copper = self.mine_and_smelt_copper::<1>();
                self.then(copper, |state, copper| {
                    let wire = CopperWireRecipe::craft(&mut state.tick, (copper,)).0;
                    state.queue.set_already_resolved_handle(wire)
                })
            })
            .collect_vec();
        let wire_bundles = self.join(wire_bundles);
        let both = self.pair(wire_bundles, iron);
        let assembler = self.then(both, |state, (wire_bundles, iron)| {
            let mut copper_wire = Resource::new_empty();
            for bundle in wire_bundles {
                copper_wire += bundle.to_resource();
            }
            let assembler = Assembler::build(
                &state.tick,
                CopperWireRecipe,
                copper_wire.bundle().unwrap(),
                iron,
            );
            state.queue.set_already_resolved_handle(assembler)
        });
        let mut assembler = self.wait_for(assembler);

        const NUM_WIRE_CYCLES: u32 = 6;
        let copper =
            self.mine_and_smelt_copper::<{ CopperWireRecipe::INPUT_AMOUNTS.0 * NUM_WIRE_CYCLES }>();
        let iron = self.mine_and_smelt_iron();
        let copper = self.wait_for(copper);
        assembler.inputs(&self.tick).0.add(copper);
        self.advance_by(CopperWireRecipe::TIME * NUM_WIRE_CYCLES as u64);
        let copper_wire = assembler.outputs(&self.tick).0.empty().bundle().unwrap();
        let iron = self.wait_for(iron);
        let _assembler2 = Assembler::build(&self.tick, CopperWireRecipe, copper_wire, iron);

        // let iron = self.mine_and_smelt_iron();
        // let copper = self.mine_and_smelt_copper();
        // let steel_lab = Lab::build(&self.tick, &self.steel_technology, iron, copper);
        // // steel_lab.inputs(tick).

        todo!("WIP: {}", self.tick.cur())
    }
}
