use rustc_hir::{def::DefKind, def_id::DefId, Unsafety};
use rustc_middle::{
    mir::Body,
    ty::{self, FnSig, GenericArgsRef, Instance, PolyFnSig, TyCtxt},
};
use rustc_target::spec::abi::Abi;

use crate::{caching::load_local_analysis_results, local_analysis::CachedBodyAnalysis};

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
                        load_local_analysis_results::<CachedBodyAnalysis>(tcx, def_id)?;
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

/// Erases all regions in the signature since we do not care about them when performing matching.
pub fn erase_regions_in_sig<'tcx>(poly_fn_sig: PolyFnSig<'tcx>, tcx: TyCtxt<'tcx>) -> FnSig<'tcx> {
    tcx.instantiate_bound_regions_with_erased(tcx.erase_regions(poly_fn_sig))
}

/// Computes function signature of a method of Fn-like trait.
pub fn fn_trait_method_sig<'tcx>(
    item_def_id: DefId,
    item_args: GenericArgsRef<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> FnSig<'tcx> {
    let item_ty = tcx.type_of(item_def_id).instantiate(tcx, item_args);
    match item_ty.kind() {
        // Handles the case when the item is an actual method; e.g., FnOnce::call_once.
        ty::FnDef(..) => {
            // From the generics, we need to find a generic that corresponds to Self and one that
            // corresponds to Args.
            let (self_arg, args_arg) = {
                let maybe_self_ty = item_args[0].expect_ty();
                match maybe_self_ty.kind() {
                    // Generic order is swapped in the implementation of Fn traits for boxed
                    // closures :(
                    ty::Tuple(..) => {
                        let self_arg = item_args[1].expect_ty();
                        // Swap the order.
                        (self_arg, maybe_self_ty)
                    }
                    // Sometimes the Self argument can be boxed, need to unbox it.
                    _ if maybe_self_ty.is_box() => {
                        (maybe_self_ty.boxed_ty(), item_args[1].expect_ty())
                    }
                    _ => (maybe_self_ty, item_args[1].expect_ty()),
                }
            };

            // Now, determine the signature from the Self argument.
            match self_arg.kind() {
                ty::FnDef(def_id, args) => {
                    erase_regions_in_sig(tcx.fn_sig(def_id).instantiate(tcx, args), tcx)
                }
                ty::Closure(_, closure_args) => erase_regions_in_sig(
                    tcx.signature_unclosure(closure_args.as_closure().sig(), Unsafety::Normal),
                    tcx,
                ),
                ty::FnPtr(poly_fn_sig) => erase_regions_in_sig(*poly_fn_sig, tcx),
                // If we have a trait object as Self, need to use generics to reconstruct the
                // signature.
                ty::Dynamic(bounds, ..) => {
                    // Sift through the clauses on the trait object to find Self::Output.
                    let output_ty = bounds
                        .projection_bounds()
                        .find(|p| {
                            tcx.lang_items()
                                .fn_once_output()
                                .map_or(false, |id| id == p.item_def_id())
                        })
                        .map(|p| p.map_bound(|p| p.term.ty().unwrap()))
                        .unwrap();
                    // Inputs are provided as Args generic.
                    let inputs = tcx.erase_regions(args_arg.tuple_fields());
                    let output =
                        tcx.instantiate_bound_regions_with_erased(tcx.erase_regions(output_ty));
                    tcx.mk_fn_sig(inputs, output, false, Unsafety::Normal, Abi::Rust)
                }
                _ => bug!(),
            }
        }
        // Sometimes closures can be a part of the vtable, since they can implicitly implement Fn
        // and FnMut. We can extract the signature for them directly.
        ty::Closure(_, closure_args) => erase_regions_in_sig(
            tcx.signature_unclosure(closure_args.as_closure().sig(), Unsafety::Normal),
            tcx,
        ),
        _ => bug!(),
    }
}
