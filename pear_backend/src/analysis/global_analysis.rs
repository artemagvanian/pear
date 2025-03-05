use rustc_middle::ty::TyCtxt;

pub trait GlobalAnalysis<'tcx> {
    fn perform_analysis(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation;
}
