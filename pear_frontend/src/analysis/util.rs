use rustc_hir::def::DefKind;
use rustc_middle::{
    mir::Body,
    ty::{self, Instance, TyCtxt},
};

use super::CachedBodyAnalysis;
use pear_backend::LocalAnalysis;

#[allow(unused)]
pub fn substituted_mir<'tcx>(
    instance: &Instance<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> Result<Body<'tcx>, String> {
    let instance_body = match instance.def {
        ty::InstanceDef::Item(def) => {
            let def_kind = tcx.def_kind(def);
            match def_kind {
                DefKind::Const
                | DefKind::Static(..)
                | DefKind::AssocConst
                | DefKind::Ctor(..)
                | DefKind::AnonConst
                | DefKind::InlineConst => tcx.mir_for_ctfe(def).clone(),
                _ => {
                    let def_id = instance.def_id();
                    let cached_body =
                        <CachedBodyAnalysis as LocalAnalysis>::load_local_analysis_results(
                            tcx, def_id,
                        )?;
                    tcx.erase_regions(cached_body.owned_body())
                }
            }
        }
        ty::InstanceDef::Virtual(..) | ty::InstanceDef::Intrinsic(..) => {
            return Err("instance {instance:?} does not have callable mir".to_string());
        }
        ty::InstanceDef::VTableShim(..)
        | ty::InstanceDef::ReifyShim(..)
        | ty::InstanceDef::FnPtrShim(..)
        | ty::InstanceDef::ClosureOnceShim { .. }
        | ty::InstanceDef::DropGlue(..)
        | ty::InstanceDef::CloneShim(..)
        | ty::InstanceDef::ThreadLocalShim(..)
        | ty::InstanceDef::FnPtrAddrShim(..) => tcx.mir_shims(instance.def).clone(),
    };
    Ok(instance
        .try_instantiate_mir_and_normalize_erasing_regions(
            tcx,
            ty::ParamEnv::reveal_all(),
            ty::EarlyBinder::bind(instance_body.clone()),
        )
        .unwrap_or(instance_body))
}
