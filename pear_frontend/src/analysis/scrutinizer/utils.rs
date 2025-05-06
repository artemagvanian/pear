use rustc_hir::def::DefKind;
use rustc_middle::ty::{FnSig, Instance, TyCtxt};

pub fn instance_sig<'tcx>(instance: Instance<'tcx>, tcx: TyCtxt<'tcx>) -> FnSig<'tcx> {
    if matches!(tcx.def_kind(instance.def_id()), DefKind::Closure) {
        tcx.instantiate_bound_regions_with_erased(
            tcx.erase_regions(instance.args.as_closure().sig()),
        )
    } else {
        tcx.instantiate_bound_regions_with_erased(
            tcx.erase_regions(
                tcx.fn_sig(instance.def_id())
                    .instantiate(tcx, instance.args),
            ),
        )
    }
}
