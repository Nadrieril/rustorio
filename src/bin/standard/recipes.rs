use crate::*;

/// Trait implemented only for `Bundle<R, N>`. Used to avoid a generic const param in some
/// situations.
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

/// A tuple of `Bundle<R, N>`.
pub trait MultiBundle: Sized {
    /// `Self` represented as a tuple. That's to correct for inconsistencies in representation.
    type AsTuple: MultiBundle<AsResource = Self::AsResource>;
    /// The corresponding tuple of `Resource<R>`.
    type AsResource;

    /// Count the number of bundle tuples available in the given resource tuple.
    fn bundle_count(res: &Self::AsResource) -> u32;
    /// Add the bundle tuple to the resource tuple.
    fn add(res: &mut Self::AsResource, bundle: Self);
    /// Pop a bundle tuple from a resource tuple, if there are enough resources.
    fn bundle(res: &mut Self::AsResource) -> Option<Self>;
}

// That's the inconsistent case.
impl<R1: ResourceType, const N1: u32> MultiBundle for Bundle<R1, N1> {
    type AsTuple = (Self,);
    type AsResource = (Resource<R1>,);

    fn bundle_count(res: &Self::AsResource) -> u32 {
        res.0.amount() / N1
    }
    fn add(res: &mut Self::AsResource, bundle: Self) {
        res.0 += bundle;
    }
    fn bundle(res: &mut Self::AsResource) -> Option<Self> {
        Some(res.0.bundle().ok()?)
    }
}
impl<R1: ResourceType, const N1: u32> MultiBundle for (Bundle<R1, N1>,) {
    type AsTuple = Self;
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
    type AsTuple = Self;
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

/// Trait to compute statically-counted inputs and outputs.
pub trait ConstRecipe: Recipe + Any {
    type BundledInputs: MultiBundle<AsResource = Self::Inputs>;
    type BundledOutputs: MultiBundle<AsResource = Self::Outputs>;
}
impl<R> ConstRecipe for R
where
    R: Recipe + Any,
    R: RecipeEx<InputBundle: MultiBundle<AsResource = Self::Inputs>>,
    R: RecipeEx<OutputBundle: MultiBundle<AsResource = Self::Outputs>>,
{
    type BundledInputs = <R as RecipeEx>::InputBundle;
    // Correct for inconsistencies in tuple-wrapping single outputs.
    type BundledOutputs = <<R as RecipeEx>::OutputBundle as MultiBundle>::AsTuple;
}
