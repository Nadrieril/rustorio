use crate::*;

pub trait IsBundle {
    const AMOUNT: u32;
    type Resource: ResourceType;
    fn to_resource(self) -> Resource<Self::Resource>;
}
impl<const AMOUNT: u32, R: ResourceType> IsBundle for Bundle<R, AMOUNT> {
    const AMOUNT: u32 = AMOUNT;
    type Resource = R;
    fn to_resource(self) -> Resource<Self::Resource> {
        self.to_resource()
    }
}
impl<const AMOUNT: u32, R: ResourceType> IsBundle for (Bundle<R, AMOUNT>,) {
    const AMOUNT: u32 = AMOUNT;
    type Resource = R;
    fn to_resource(self) -> Resource<Self::Resource> {
        self.0.to_resource()
    }
}

/// A tuple of `Bundle<R, N>`.
pub trait MultiBundle: Sized {
    /// The corresponding tuple of `Resource<R>`.
    type AsResource;

    /// Count the number of bundle tuples available in the given resource tuple.
    fn bundle_count(res: &Self::AsResource) -> u32;
    /// Add the bundle tuple to the resource tuple.
    fn add(res: &mut Self::AsResource, bundle: Self);
    /// Pop a bundle tuple from a resource tuple, if there are enough resources.
    fn bundle(res: &mut Self::AsResource) -> Option<Self>;
}

impl<R1: ResourceType, const N1: u32> MultiBundle for (Bundle<R1, N1>,) {
    type AsResource = (Resource<R1>,);

    fn bundle_count(res: &Self::AsResource) -> u32 {
        res.0.amount() / N1
    }
    fn add(res: &mut Self::AsResource, bundle: Self) {
        res.0 += bundle.0;
    }
    fn bundle(res: &mut Self::AsResource) -> Option<Self> {
        Some((res.0.bundle().ok()?,))
    }
}
impl<R1: ResourceType, const N1: u32, R2: ResourceType, const N2: u32> MultiBundle
    for (Bundle<R1, N1>, Bundle<R2, N2>)
{
    type AsResource = (Resource<R1>, Resource<R2>);

    fn bundle_count(res: &Self::AsResource) -> u32 {
        std::cmp::min(res.0.amount() / N1, res.1.amount() / N2)
    }
    fn add(res: &mut Self::AsResource, bundle: Self) {
        res.0 += bundle.0;
        res.1 += bundle.1;
    }
    fn bundle(res: &mut Self::AsResource) -> Option<Self> {
        if res.0.amount() >= N1 && res.1.amount() >= N2 {
            Some((res.0.bundle().ok()?, res.1.bundle().ok()?))
        } else {
            None
        }
    }
}

// Const fns because direct field access is not allowed in const exprs.
pub const fn tup1_field0<A: Copy>(x: (A,)) -> A {
    x.0
}
pub const fn tup2_field0<A: Copy, B: Copy>(x: (A, B)) -> A {
    x.0
}
pub const fn tup2_field1<A: Copy, B: Copy>(x: (A, B)) -> B {
    x.1
}

/// Trait to compute statically-counted inputs and outputs. The const generic is needed because the
/// impls would otherwise be considered to overlap.
pub trait ConstRecipeImpl<const INPUT_N: u32>: Recipe {
    type BundledInputs_: MultiBundle<AsResource = Self::Inputs>;
    type BundledOutputs_: MultiBundle<AsResource = Self::Outputs>;
}

impl<R, I, O> ConstRecipeImpl<1> for R
where
    I: ResourceType,
    O: ResourceType,
    R: Recipe<Inputs = (Resource<I>,), InputAmountsType = (u32,)>,
    R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
    [(); { tup1_field0(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup1_field0(R::OUTPUT_AMOUNTS) } as usize]:,
{
    type BundledInputs_ = (Bundle<I, { tup1_field0(R::INPUT_AMOUNTS) }>,);
    type BundledOutputs_ = (Bundle<O, { tup1_field0(R::OUTPUT_AMOUNTS) }>,);
}

impl<R, I1, I2, O> ConstRecipeImpl<2> for R
where
    I1: ResourceType,
    I2: ResourceType,
    O: ResourceType,
    R: Recipe<Inputs = (Resource<I1>, Resource<I2>), InputAmountsType = (u32, u32)>,
    R: Recipe<Outputs = (Resource<O>,), OutputAmountsType = (u32,)>,
    [(); { tup2_field0(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup2_field1(R::INPUT_AMOUNTS) } as usize]:,
    [(); { tup1_field0(R::OUTPUT_AMOUNTS) } as usize]:,
{
    type BundledInputs_ = (
        Bundle<I1, { tup2_field0(R::INPUT_AMOUNTS) }>,
        Bundle<I2, { tup2_field1(R::INPUT_AMOUNTS) }>,
    );
    type BundledOutputs_ = (Bundle<O, { tup1_field0(R::OUTPUT_AMOUNTS) }>,);
}

/// Provides the `INPUT_N` to pass to `ConstRecipe`. This is weird af but seems to work.
/// Can't define generic impls for this of course, since rustc considers them to overlap.
pub trait InputN {
    const INPUT_N: u32;
}
impl InputN for IronSmelting {
    const INPUT_N: u32 = 1;
}
impl InputN for CopperSmelting {
    const INPUT_N: u32 = 1;
}
impl InputN for SteelSmelting {
    const INPUT_N: u32 = 1;
}
impl InputN for CopperWireRecipe {
    const INPUT_N: u32 = 1;
}
impl InputN for TechRecipe<SteelTechnology> {
    const INPUT_N: u32 = 1;
}
impl InputN for TechRecipe<PointsTechnology> {
    const INPUT_N: u32 = 1;
}
impl InputN for ElectronicCircuitRecipe {
    const INPUT_N: u32 = 2;
}
impl InputN for RedScienceRecipe {
    const INPUT_N: u32 = 2;
}
impl InputN for PointRecipe {
    const INPUT_N: u32 = 2;
}

/// Trait to compute statically-counted inputs and outputs.
pub trait ConstRecipe: Recipe + InputN + Any {
    type BundledInputs: MultiBundle<AsResource = Self::Inputs>;
    type BundledOutputs: MultiBundle<AsResource = Self::Outputs>;
}
impl<R: Recipe + InputN + Any + ConstRecipeImpl<{ R::INPUT_N }>> ConstRecipe for R {
    type BundledInputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledInputs_;
    type BundledOutputs = <R as ConstRecipeImpl<{ R::INPUT_N }>>::BundledOutputs_;
}
