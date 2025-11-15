/// This is a stub of the kani_middle::transform::BodyTransformation type. 
/// In Kani, all the [`Body`]s are instrumented at collection time; the
/// BodyTransformation object is used to retrieve a transformed Instance body.
/// The transformations optimize and instrument the retrieved bodies. 
/// 
/// To remove this from the collector module, 
/// we will need to determine if the caching would have any meaningful benefit 
/// in our case, i.e., is there another cache whose work we're duplicating. 
/// 
/// For now, we're just going to keep this to leave the Kani collector as intact as possible. 
/// 

use rustc_middle::ty::TyCtxt;
use rustc_public::mir::Body;
use rustc_public::mir::mono::Instance;
use std::collections::HashMap;
use std::fmt::Debug;

#[derive(Debug)]
pub struct BodyTransformation {
    /// Cache transformation results.
    cache: HashMap<Instance, TransformationResult>,
}

impl BodyTransformation {
    pub fn new() -> Self {
        BodyTransformation {
            cache: Default::default(),
        }
    }

    /// Equivalent to `body()`, but avoids cloning the returned `Body`.
    pub fn body_ref(&mut self, _tcx: TyCtxt, instance: Instance) -> &Body {
        &self
            .cache
            .entry(instance)
            .or_insert_with(|| {
                // Add to the cache if there's no existing entry.
                let body = instance.body().unwrap();

                TransformationResult(body)
            })
        .0
    }

    /// Retrieve the body of an instance. This does not apply global passes, but will retrieve the
    /// body after global passes running if they were previously applied.
    ///
    /// Note that this assumes that the instance does have a body since existing consumers already
    /// assume that. Use `instance.has_body()` to check if an instance has a body.
    pub fn body(&mut self, tcx: TyCtxt, instance: Instance) -> Body {
        self.body_ref(tcx, instance).clone()
    }
}

#[derive(Clone, Debug)]
struct TransformationResult(Body);

// #[allow(dead_code)]
impl TransformationResult {
    /// The original TransformationResult contained a bool indicating
    /// whether the Body had been modified. This is always false now.
    pub fn has_been_modified(&self) -> bool {
        false
    }
}