#![forbid(unsafe_code)]
#![allow(unused)]

use rustorio::{
    self, Bundle, HandRecipe, Recipe, Resource, ResourceType, Tick,
    buildings::{Assembler, Furnace, Lab},
    gamemodes::Standard,
    recipes::{CopperSmelting, CopperWireRecipe, FurnaceRecipe, IronSmelting},
    research::SteelTechnology,
    resources::{Copper, CopperOre, Iron, IronOre, Point},
    territory::{MINING_TICK_LENGTH, Miner, Territory},
};

type GameMode = Standard;

type StartingResources = <GameMode as rustorio::GameMode>::StartingResources;

fn main() {
    fn user_main(
        mut tick: Tick,
        starting_resources: StartingResources,
    ) -> (Tick, Bundle<Point, 200>) {
        GameState::new(tick, starting_resources).play()
    }
    rustorio::play::<GameMode>(user_main);
}

trait RecipeSingleInput: Recipe<Inputs = (Resource<Self::SingleInput>,)> {
    type SingleInput: ResourceType;
}
impl<I: ResourceType, R: Recipe<Inputs = (Resource<I>,)>> RecipeSingleInput for R {
    type SingleInput = I;
}

trait RecipeSingleOutput: Recipe<Outputs = (Resource<Self::SingleOutput>,)> {
    type SingleOutput: ResourceType;
}
impl<I: ResourceType, R: Recipe<Outputs = (Resource<I>,)>> RecipeSingleOutput for R {
    type SingleOutput = I;
}

struct GameState {
    tick: Tick,
    iron_territory: Territory<IronOre>,
    copper_territory: Territory<CopperOre>,
    steel_technology: SteelTechnology,
    iron: Resource<Iron>,
    copper: Resource<Copper>,
    hand_iron_furnace: Option<Furnace<IronSmelting>>,
    hand_copper_furnace: Option<Furnace<CopperSmelting>>,
}

fn mine_and_smelt<const COUNT: u32, R>(
    tick: &mut Tick,
    territory: &mut Territory<R::SingleInput>,
    furnace: &mut Furnace<R>,
) -> Bundle<R::SingleOutput, COUNT>
where
    R: FurnaceRecipe + RecipeSingleInput + RecipeSingleOutput,
{
    let ore: Bundle<_, COUNT> = if territory.num_miners() == 0 {
        territory.hand_mine(tick)
    } else {
        loop {
            match territory.resources(tick).bundle() {
                Ok(x) => break x,
                Err(_) => tick.advance_by(MINING_TICK_LENGTH),
            }
        }
    };
    furnace.inputs(&tick).0.add(ore);
    tick.advance_by(COUNT as u64 * R::TIME);
    furnace.outputs(&tick).0.empty().bundle::<COUNT>().unwrap()
}

impl GameState {
    fn new(mut tick: Tick, starting_resources: StartingResources) -> Self {
        let StartingResources {
            iron,
            iron_territory,
            copper_territory,
            steel_technology,
        } = starting_resources;

        tick.log(true);
        GameState {
            tick,
            iron_territory,
            copper_territory,
            steel_technology,
            iron: iron.to_resource(),
            copper: Resource::new_empty(),
            hand_iron_furnace: Default::default(),
            hand_copper_furnace: Default::default(),
        }
    }

    fn make_furnace<R: FurnaceRecipe>(&mut self, r: R) -> Furnace<R> {
        let iron = self.iron.bundle().unwrap();
        Furnace::build(&self.tick, r, iron)
    }

    fn mine_and_smelt_iron<const COUNT: u32>(&mut self) -> Bundle<Iron, COUNT> {
        mine_and_smelt::<COUNT, _>(
            &mut self.tick,
            &mut self.iron_territory,
            self.hand_iron_furnace.as_mut().unwrap(),
        )
    }

    fn mine_and_smelt_copper<const COUNT: u32>(&mut self) -> Bundle<Copper, COUNT> {
        mine_and_smelt::<COUNT, _>(
            &mut self.tick,
            &mut self.copper_territory,
            self.hand_copper_furnace.as_mut().unwrap(),
        )
    }

    fn play(mut self) -> (Tick, Bundle<Point, 200>) {
        let iron = self.iron.bundle().unwrap();
        self.hand_iron_furnace = Some(Furnace::build(&self.tick, IronSmelting, iron));

        let iron = self.mine_and_smelt_iron();
        self.hand_copper_furnace = Some(Furnace::build(&self.tick, CopperSmelting, iron));

        let iron = self.mine_and_smelt_iron();
        let copper = self.mine_and_smelt_copper();
        self.iron_territory
            .add_miner(&self.tick, Miner::build(iron, copper));

        let iron = self.mine_and_smelt_iron();
        let copper = self.mine_and_smelt_copper();
        self.copper_territory
            .add_miner(&self.tick, Miner::build(iron, copper));

        let mut copper_wire = Resource::new_empty();
        for i in 0..6 {
            let copper = self.mine_and_smelt_copper();
            copper_wire += CopperWireRecipe::craft(&mut self.tick, (copper,))
                .0
                .to_resource();
        }
        let iron = self.mine_and_smelt_iron();
        let mut assembler = Assembler::build(
            &self.tick,
            CopperWireRecipe,
            copper_wire.bundle().unwrap(),
            iron,
        );

        const NUM_WIRE_CYCLES: u32 = 6;
        let copper =
            self.mine_and_smelt_copper::<{ CopperWireRecipe::INPUT_AMOUNTS.0 * NUM_WIRE_CYCLES }>();
        assembler.inputs(&self.tick).0.add(copper);
        self.tick
            .advance_by(CopperWireRecipe::TIME * NUM_WIRE_CYCLES as u64);
        let copper_wire = assembler.outputs(&self.tick).0.empty().bundle().unwrap();
        let iron = self.mine_and_smelt_iron();
        let mut assembler2 = Assembler::build(&self.tick, CopperWireRecipe, copper_wire, iron);

        // let iron = self.mine_and_smelt_iron();
        // let copper = self.mine_and_smelt_copper();
        // let steel_lab = Lab::build(&self.tick, &self.steel_technology, iron, copper);
        // // steel_lab.inputs(tick).

        todo!("WIP: {}", self.tick.cur())
    }
}
