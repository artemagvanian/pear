use itertools::Itertools;
use rustc_hir::{def_id::DefId, Unsafety};
use rustc_middle::ty::{self, FnSig, GenericArgsRef, PolyFnSig, TyCtxt};
use rustc_target::spec::abi::Abi;

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
    let item_args = item_args
        .iter()
        .filter_map(|arg| arg.as_type())
        .collect_vec();
    match item_ty.kind() {
        // Handles the case when the item is an actual method; e.g., FnOnce::call_once.
        ty::FnDef(..) => {
            // From the generics, we need to find a generic that corresponds to Self and one that
            // corresponds to Args.
            let (self_arg, args_arg) = {
                let maybe_self_ty = item_args[0];
                match maybe_self_ty.kind() {
                    // Generic order is swapped in the implementation of Fn traits for boxed
                    // closures :(
                    ty::Tuple(..) => {
                        let self_arg = item_args[1];
                        // Swap the order.
                        (self_arg, maybe_self_ty)
                    }
                    // Sometimes the Self argument can be boxed, need to unbox it.
                    _ if maybe_self_ty.is_box() => (maybe_self_ty.boxed_ty(), item_args[1]),
                    // Sometimes the Self argument can be a ref, need to deref it.
                    _ if maybe_self_ty.is_ref() => (maybe_self_ty.peel_refs(), item_args[1]),
                    _ => (maybe_self_ty, item_args[1]),
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
                _ => bug!("{:?}", self_arg.kind()),
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
