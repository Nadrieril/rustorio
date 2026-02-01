#![forbid(unsafe_code)]
#![allow(unused)]

use rustorio::{
    self, Bundle, Recipe, Tick,
    buildings::Furnace,
    gamemodes::Tutorial,
    recipes::{CopperSmelting, IronSmelting},
    resources::Copper,
    territory::MINING_TICK_LENGTH,
};

type GameMode = Tutorial;

type StartingResources = <GameMode as rustorio::GameMode>::StartingResources;

fn main() {
    rustorio::play::<GameMode>(user_main);
}

fn user_main(mut tick: Tick, starting_resources: StartingResources) -> (Tick, Bundle<Copper, 4>) {
    tick.log(true);

    let StartingResources {
        iron,
        mut iron_territory,
        mut copper_territory,
        guide,
    } = starting_resources;

    // To start, run the game using `rustorio play tutorial` (or whatever this save is called), and follow the hint.
    // If you get stuck, try giving the guide other objects you've found, like the `tick` object.
    let mut furnace = Furnace::build(&tick, CopperSmelting, iron);

    for _ in 0..4 {
        let copper_ore = copper_territory.hand_mine::<1>(&mut tick);
        furnace.inputs(&tick).0.add(copper_ore);
    }

    loop {
        match furnace.outputs(&tick).0.bundle() {
            Ok(copper) => return (tick, copper),
            Err(_) => tick.advance(),
        }
    }
}
