use itertools::Itertools;
use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::{Body, Local, Terminator, TerminatorKind},
    ty::TyCtxt,
};

use flowistry::{
    infoflow::{Direction, FlowAnalysis},
    mir::{engine, placeinfo::PlaceInfo},
};

use rustc_utils::mir::location_or_arg::LocationOrArg;

use crate::analysis::scrutinizer::scrutinizer_local::ScrutinizerBody;

// This function computes all locals that depend on the argument local for a given def_id.
pub fn compute_dependent_terminators<'tcx>(
    def_id: DefId,
    important_args: Vec<Local>,
    body_with_facts: ScrutinizerBody<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> Vec<Terminator<'tcx>> {
    let body_with_facts_ref: &'tcx ScrutinizerBody<'tcx> =
        unsafe { std::mem::transmute(&body_with_facts) };
    let place_info = PlaceInfo::build(tcx, def_id, body_with_facts_ref);
    let location_domain = place_info.location_domain().clone();

    let (body, _) = body_with_facts.clone().split();
    let body_ref: &'tcx Body<'tcx> = unsafe { std::mem::transmute(&body) };

    let results = {
        let analysis = FlowAnalysis::new(tcx, def_id, body_ref, place_info);
        engine::iterate_to_fixpoint(tcx, &body, location_domain, analysis)
    };

    let dependent_terminators = body
        .basic_blocks
        .iter_enumerated()
        .filter_map(|(bb_idx, bb)| {
            let terminator = bb.terminator();
            let terminator_loc = body.terminator_loc(bb_idx);
            match &terminator.kind {
                TerminatorKind::Call { args, .. } => {
                    let targets = args
                        .iter()
                        .filter_map(|arg| arg.place())
                        .map(|place| (place, LocationOrArg::Location(terminator_loc)))
                        .collect_vec();
                    flowistry::infoflow::compute_dependencies(
                        &results,
                        vec![targets],
                        Direction::Backward,
                    )[0]
                    .iter()
                    .any(|location_or_arg| {
                        if let LocationOrArg::Arg(local) = *location_or_arg {
                            important_args.contains(&local)
                        } else {
                            false
                        }
                    })
                    .then_some(terminator.clone())
                }
                TerminatorKind::Drop { place, .. } => {
                    let targets = vec![(*place, LocationOrArg::Location(terminator_loc))];
                    flowistry::infoflow::compute_dependencies(
                        &results,
                        vec![targets],
                        Direction::Backward,
                    )[0]
                    .iter()
                    .any(|location_or_arg| {
                        if let LocationOrArg::Arg(local) = *location_or_arg {
                            important_args.contains(&local)
                        } else {
                            false
                        }
                    })
                    .then_some(terminator.clone())
                }
                _ => None,
            }
        })
        .collect();

    drop(body_with_facts);
    drop(body);

    dependent_terminators
}
