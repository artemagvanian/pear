use rustc_hir::def::DefKind;
use rustc_middle::ty::{FnSig, Instance, TyCtxt};

pub fn instance_sig<'tcx>(instance: Instance<'tcx>, tcx: TyCtxt<'tcx>) -> FnSig<'tcx> {
    tcx.instantiate_bound_regions_with_erased(
        tcx.erase_regions(
            tcx.fn_sig(instance.def_id())
                .instantiate(tcx, instance.args),
        ),
    )
}

pub fn num_args_for_instance<'tcx>(instance: Instance<'tcx>, tcx: TyCtxt<'tcx>) -> usize {
    if matches!(tcx.def_kind(instance.def_id()), DefKind::Closure) {
        instance.args.as_closure().sig().inputs().skip_binder().len()
    } else {
        instance_sig(instance, tcx).inputs().len()
    }
}
