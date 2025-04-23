use rustc_middle::{
    bug,
    ty::{FnSig, Instance, TyCtxt},
};

pub fn instance_sig<'tcx>(instance: Instance<'tcx>, tcx: TyCtxt<'tcx>) -> FnSig<'tcx> {
    if tcx.is_closure_or_coroutine(instance.def_id()) {
        if tcx.is_coroutine(instance.def_id()) {
            bug!("coroutines do not have a conventional signature");
        }
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

pub fn num_args_for_instance<'tcx>(instance: Instance<'tcx>, tcx: TyCtxt<'tcx>) -> usize {
    instance_sig(instance, tcx).inputs().len()
}
