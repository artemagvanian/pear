use rustc_middle::mir::{visit::Visitor, Location, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;
use std::collections::HashSet;

struct CallCrateCollector<'tcx> {
    crates_from_calls: HashSet<String>,
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> Visitor<'tcx> for CallCrateCollector<'tcx> {
    fn visit_terminator(&mut self, terminator: &Terminator<'tcx>, _location: Location) {
        if let TerminatorKind::Call { func, .. } = &terminator.kind {
            if let Some((callee_def_id, _)) = func.const_fn_def() {
                self.crates_from_calls
                    .insert(self.tcx.crate_name(callee_def_id.krate).to_string());
            }
        }
    }
}
