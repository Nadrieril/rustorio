use crate::*;

/// A store of various resources.
#[derive(Default)]
pub struct Resources {
    any: HashMap<TypeId, Box<dyn Any>>,
}

impl Resources {
    fn or_insert_any<X: Any>(&mut self, f: impl FnOnce() -> X) -> &mut X {
        let storage: &mut (dyn Any + 'static) = self
            .any
            .entry(TypeId::of::<X>())
            .or_insert_with(|| Box::new(f()))
            .as_mut();
        storage.downcast_mut().unwrap()
    }

    pub fn resource<R: ResourceType + Any>(&mut self) -> &mut Resource<R> {
        self.or_insert_any(|| Resource::<R>::new_empty())
    }
    pub fn reusable<T: Reusable + Any>(&mut self) -> &mut ReusableContainer<T> {
        self.or_insert_any(|| ReusableContainer::empty())
    }
}

/// Items that can be made use of many times (e.g. recipes).
#[marker]
pub trait Reusable {}

impl<T: Technology> Reusable for T {}
impl<T: Recipe> Reusable for T {}

/// Token that indicates that the given reusable resource is available.
pub struct Available<T: Reusable>(PhantomData<T>);

impl<T: Reusable> Copy for Available<T> {}
impl<T: Reusable> Clone for Available<T> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

/// A container for a reusable resource. The `Available` token serves as access to the contained
/// data.
pub struct ReusableContainer<T> {
    /// The reusable value, once we've built it.
    val: Option<T>,
}

impl<T: Reusable> ReusableContainer<T> {
    pub fn empty() -> Self {
        Self { val: None }
    }
    pub fn available(&self) -> Option<Available<T>> {
        self.val.as_ref().map(|_| Available(PhantomData))
    }
    pub fn get(&self, _: Available<T>) -> &T {
        self.val.as_ref().unwrap()
    }
    pub fn take(&mut self, _: Available<T>) -> T {
        self.val.take().unwrap()
    }
    pub fn set(&mut self, t: T) -> Available<T> {
        self.val = Some(t);
        Available(PhantomData)
    }
}

// /// A resource along with a queue of sinks waiting for that resource.
// pub struct ResourceWithQueue<R: ResourceType> {
//     resource: Resource<R>,
//     /// Keep sorted by priority.
//     queue: VecDeque<ResourceWaiter<R>>,
// }

// struct ResourceWaiter<R: ResourceType> {
//     sink: Sink<Resource<R>>,
//     /// The quantity of resource expected.
//     quantity: u32,
//     priority: Priority,
// }

// impl<R: ResourceType + Any> ResourceWithQueue<R> {
//     pub fn new() -> Self {
//         Self {
//             resource: Resource::new_empty(),
//             queue: Default::default(),
//         }
//     }

//     /// Add some resource to the pool.
//     pub fn add<const N: u32>(&mut self, waiters: &mut CallBackQueue, bundle: Bundle<R, N>) {
//         self.resource.add(bundle);
//         while let Some(w) = self.queue.front()
//             && let Ok(bundle) = self.resource.split_off(w.quantity)
//         {
//             let w = self.queue.pop_front().unwrap();
//             w.sink.give(waiters, bundle);
//         }
//     }

//     /// Add some resource to the pool without trying to wake the waiters.
//     pub fn add_no_wake<const N: u32>(&mut self, bundle: Bundle<R, N>) {
//         self.resource.add(bundle);
//     }

//     pub fn bundle<const N: u32>(&mut self) -> Result<Bundle<R, N>, InsufficientResourceError<R>> {
//         self.resource.bundle()
//     }

//     pub fn wait_for_bundle<const N: u32>(
//         &mut self,
//         waiters: &mut CallBackQueue,
//         sink: Sink<Bundle<R, N>>,
//         p: Priority,
//     ) {
//         if self.queue.is_empty()
//             && let Ok(bundle) = self.resource.bundle()
//         {
//             sink.give(waiters, bundle);
//         } else {
//             self.queue.push_back(ResourceWaiter {
//                 sink: sink.map(|_, mut res: Resource<R>| res.bundle().unwrap()),
//                 quantity: N,
//                 priority: p,
//             });
//             self.queue
//                 .make_contiguous()
//                 .sort_by_key(|w| Reverse(w.priority));
//         }
//     }
// }
