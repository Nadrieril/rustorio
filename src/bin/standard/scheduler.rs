use std::{any::Any, cell::RefCell, collections::VecDeque, rc::Rc};

use crate::*;

pub struct Sink<T, S = CallBackQueue>(Box<dyn FnOnce(&mut S, T)>);
pub type StateSink<T> = Sink<T, GameState>;

impl<T: Any, S: Any> Sink<T, S> {
    pub fn from_fn(f: impl FnOnce(&mut S, T) + 'static) -> Self {
        Self(Box::new(f))
    }

    pub fn give(self, s: &mut S, x: T) {
        (self.0)(s, x)
    }
    pub fn map<U: Any>(self, f: impl FnOnce(&mut S, U) -> T + 'static) -> Sink<U, S> {
        Sink::from_fn(|s, u| {
            let t = f(s, u);
            self.give(s, t);
        })
    }
    pub fn map_state<Q: Any>(self, f: impl FnOnce(&mut Q) -> &mut S + 'static) -> Sink<T, Q> {
        Sink::from_fn(|s, x| {
            self.give(f(s), x);
        })
    }
}
impl<T: Any> Sink<T, CallBackQueue> {
    pub fn with_gamestate(self) -> Sink<T, GameState> {
        self.map_state::<GameState>(|s| &mut s.queue)
    }
}

impl<A: Any, B: Any> Sink<(A, B)> {
    pub fn split(self) -> (Sink<A>, Sink<B>) {
        struct PairinatorInner<A, B> {
            a: Option<A>,
            b: Option<B>,
            sink: Sink<(A, B)>,
        }

        fn call_back_if_ready<A: Any, B: Any>(
            rc: Rc<RefCell<PairinatorInner<A, B>>>,
            q: &mut CallBackQueue,
        ) {
            if let Some(inner) = Rc::into_inner(rc) {
                let inner = RefCell::into_inner(inner);
                inner.sink.give(q, (inner.a.unwrap(), inner.b.unwrap()))
            }
        }
        let rc = Rc::new(RefCell::new(PairinatorInner {
            a: None,
            b: None,
            sink: self,
        }));
        let a_side = rc.clone();
        let a = Sink::from_fn(|q, x| {
            a_side.borrow_mut().a = Some(x);
            call_back_if_ready(a_side, q);
        });
        let b_side = rc;
        let b = Sink::from_fn(|q, x| {
            b_side.borrow_mut().b = Some(x);
            call_back_if_ready(b_side, q);
        });
        (a, b)
    }
}

impl<const N: usize, T: Any> Sink<[T; N]> {
    pub fn split_n(self) -> [Sink<T>; N] {
        let rc = Rc::new(RefCell::new((vec![], self)));
        std::array::from_fn(|_| {
            let rc = rc.clone();
            Sink::from_fn(move |q, x| {
                rc.borrow_mut().0.push(x);
                if let Some(inner) = Rc::into_inner(rc) {
                    let inner = RefCell::into_inner(inner);
                    inner.1.give(q, inner.0.try_into().ok().unwrap())
                }
            })
        })
    }
}

impl<const COUNT: u32, R: ResourceType + Any> Sink<Bundle<R, COUNT>> {
    pub fn split_resource<B: IsBundle<Resource = R> + Any>(
        self,
    ) -> [Sink<B>; (COUNT / B::AMOUNT) as usize]
    where
        [(); (COUNT / B::AMOUNT) as usize]:,
    {
        assert_eq!(COUNT % B::AMOUNT, 0);
        let rc = Rc::new(RefCell::new((Resource::new_empty(), Some(self))));
        std::array::from_fn(|_| {
            let rc = rc.clone();
            Sink::from_fn(move |q, b: B| {
                let mut inner = rc.borrow_mut();
                inner.0.add(b.to_resource());
                if inner.0.amount() >= COUNT {
                    inner.1.take().unwrap().give(q, inner.0.bundle().unwrap())
                }
            })
        })
    }
}

/// One end of a pipe. `make_pipe` produces a source and a sink. When the sink is fed a value, the
/// source will make it available.
pub struct Source<T, S = CallBackQueue>(Rc<RefCell<SourceInner<T, S>>>);
struct SourceInner<T, S> {
    sink: Option<Sink<T, S>>,
    value: Option<T>,
}

impl<T: Any, S: Any> Source<T, S> {
    /// Build a pipe. When the sink is fed a value, the source will make it available.
    pub fn make_pipe() -> (Source<T, S>, Sink<T, S>) {
        Self::flexible_pipe(|sink| sink)
    }
    /// Flexible pipe that allows mapping between state types. The sink returned by this only uses
    /// the function provided if we can't resolve instantly, which makes it different from mapping
    /// the sink returned by `make_pipe` or mapping the sink passed to `Source::set_sink`.
    pub fn flexible_pipe<Q: Any>(
        f: impl FnOnce(Sink<T, S>) -> Sink<T, Q> + 'static,
    ) -> (Source<T, S>, Sink<T, Q>) {
        let rc = Rc::new(RefCell::new(SourceInner {
            sink: None,
            value: None,
        }));
        let source = Source(rc.clone());
        let sink = Sink::from_fn(move |s, x| {
            let mut inner = rc.borrow_mut();
            if inner.value.is_some() {
                panic!()
            }
            if let Some(sink) = inner.sink.take() {
                f(sink).give(s, x);
            } else {
                inner.value = Some(x);
            }
        });
        (source, sink)
    }
    pub fn make_resolved(x: T) -> Source<T, S> {
        let rc = Rc::new(RefCell::new(SourceInner {
            sink: None,
            value: Some(x),
        }));
        Source(rc)
    }
    // TODO: bad API
    pub fn get(&self) -> Option<T> {
        self.0.borrow_mut().value.take()
    }
    pub fn try_get(self) -> Result<T, Self> {
        let opt_value = self.0.borrow_mut().value.take();
        opt_value.ok_or(self)
    }
    pub fn set_sink(self, s: &mut S, sink: Sink<T, S>) {
        let mut inner = self.0.borrow_mut();
        if inner.sink.is_some() {
            panic!()
        }
        if let Some(x) = inner.value.take() {
            sink.give(s, x);
        } else {
            inner.sink = Some(sink);
        }
    }
}
impl<T: Any, S: Any> Source<Source<T, S>, S> {
    pub fn flatten(self, s: &mut S) -> Source<T, S> {
        let (source, sink) = Source::make_pipe();
        self.set_sink(
            s,
            Sink::from_fn(|s, inner: Source<T, S>| {
                inner.set_sink(s, sink);
            }),
        );
        source
    }
}

pub type WakeHandle<T> = Source<T, GameState>;

#[derive(Default)]
pub struct CallBackQueue {
    /// Waiters in need of update, e.g. because the waiter they were waiting on got completed.
    needs_call: VecDeque<Box<dyn FnOnce(&mut GameState)>>,
}

impl CallBackQueue {
    /// A `WakeHandle` normally works with a `Sink<T, GameState>`. Our producers can't feed such a
    /// sink though, because they're part of the game state and thus don't have a `&mut GameState`
    /// around. To circumvent that, this makes a `Sink<T, CallBackQueue>` that adds the real sink
    /// to a the callback queue to be resolved when the producer is done.
    pub fn detached_pipe<T: Any>(&mut self) -> (WakeHandle<T>, Sink<T>) {
        WakeHandle::flexible_pipe(|state_sink| {
            Sink::from_fn(|q: &mut CallBackQueue, x| {
                q.needs_call.push_back(Box::new(|state: &mut GameState| {
                    state_sink.give(state, x);
                }))
            })
        })
    }

    /// Get the next callback to call from the callback queue.
    pub fn next_callback(&mut self) -> Option<Box<dyn FnOnce(&mut GameState)>> {
        self.needs_call.pop_front()
    }
}

impl GameState {
    pub fn nowait<T: Any>(&mut self, x: T) -> WakeHandle<T> {
        WakeHandle::make_resolved(x)
    }
    pub fn never<T: Any>(&mut self) -> WakeHandle<T> {
        self.handle_via_sink(|_, _sink| ())
    }

    pub fn handle_via_sink<T: Any>(
        &mut self,
        f: impl FnOnce(&mut GameState, Sink<T>),
    ) -> WakeHandle<T> {
        let (h, sink) = self.queue.detached_pipe();
        f(self, sink);
        h
    }

    pub fn map_to_sink<T: Any>(&mut self, h: WakeHandle<T>, sink: Sink<T>) {
        h.set_sink(self, sink.with_gamestate());
    }

    pub fn map<T: Any, U: Any>(
        &mut self,
        h: WakeHandle<T>,
        f: impl FnOnce(&mut GameState, T) -> U + 'static,
    ) -> WakeHandle<U> {
        self.handle_via_sink(|state, sink| {
            h.set_sink(state, sink.with_gamestate().map(f));
        })
    }

    /// Schedules `f` to run after `h` completes, and returns a handle to the final output.
    pub fn then<T: Any, U: Any>(
        &mut self,
        h: WakeHandle<T>,
        f: impl FnOnce(&mut GameState, T) -> WakeHandle<U> + 'static,
    ) -> WakeHandle<U> {
        self.map(h, f).flatten(self)
    }

    /// Joins the results of two handles together.
    pub fn pair<T: Any, U: Any>(
        &mut self,
        a: WakeHandle<T>,
        b: WakeHandle<U>,
    ) -> WakeHandle<(T, U)> {
        self.handle_via_sink(|state, sink| {
            let (sink_a, sink_b) = sink.split();
            state.map_to_sink(a, sink_a);
            state.map_to_sink(b, sink_b);
        })
    }

    pub fn triple<T: Any, U: Any, V: Any>(
        &mut self,
        x: WakeHandle<T>,
        y: WakeHandle<U>,
        z: WakeHandle<V>,
    ) -> WakeHandle<(T, U, V)> {
        let xy = self.pair(x, y);
        let xyz = self.pair(xy, z);
        self.map(xyz, |_, ((x, y), z)| (x, y, z))
    }

    /// Joins the results of two handles together.
    pub fn collect_n<const N: usize, T: Any>(
        &mut self,
        handles: [WakeHandle<T>; N],
    ) -> WakeHandle<[T; N]> {
        self.handle_via_sink(|state, sink| {
            let sinks = sink.split_n();
            for (sink, h) in sinks.into_iter().zip(handles) {
                state.map_to_sink(h, sink);
            }
        })
    }
}
