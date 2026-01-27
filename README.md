# Rustorio

This is my [Rustorio](https://github.com/albertsgarde/rustorio) save.
Rustorio is a Factorio-inspired crafting game that you play by writing Rust code, go check it out!

I have never found a game so perfectly crafted for hooking me.
I have gone way overboard in implementing my solution and am having a ton of fun.
Here's what I've done so far.

I started by implementing what can only be described as an async runtime+combinators for crafting
items. I have tasks (`Waiter`s) waiting in a queue (`WaiterQueue`) to be woken up; when polled they
check the game state to see if they can make progress. That got slow so they can now wait on some
known conditions (other task to complete, or resource to be produced). The basic task is to launch
a task that crafts some inputs, put them inside an assembler, and wait for it to be done.

On top of that I started abstracting over the various crafting entities (`Machine`s and
`Producer`s). Most importantly this allowed me to merge the producing entities and rework how we
handle waiting on them to produce. Each type of producing entity now has a queue onto which waiters
add themselves (`ProducerWithQueue`), to be woken up when an output bundle is ready. That got the
whole game to run much faster and also gave me visibility into what's blocked at any given time.

Parallel to that, I started noticing patterns in the way we crafted items, and with increasingly
cursed trait logic managed to abstract those out. The result is basically a query system, where
`state.make::<Resource>()` knows what inputs are needed and what machine to use to produce the given
resource. This is recursive, so the whole thing basically runs itself.

What we're missing is making the machines themselves automatically. For a given crafting entity
type, the system does load balancing: when adding inputs we punt them to the least loaded machine
(see `MachineStorage::add_inputs`). But for now we have to create the machines by hand. The hope is
that I can also get auto-scaling to work. That's my next task (at the time of writing this README,
which I'll likely forget to update).
