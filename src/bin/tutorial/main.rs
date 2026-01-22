#![forbid(unsafe_code)]
#![allow(unused)]

use rustorio::{
    self, Bundle, Recipe, Tick,
    buildings::Furnace,
    gamemodes::Tutorial,
    recipes::{CopperSmelting, IronSmelting},
    resources::Copper,
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

    let copper_ore = copper_territory.hand_mine::<4>(&mut tick);

    furnace.inputs(&tick).0.add(copper_ore);
    tick.advance_by(4 * CopperSmelting::TIME);
    let bundle = furnace.outputs(&tick).0.empty().bundle().unwrap();
    (tick, bundle)
}
