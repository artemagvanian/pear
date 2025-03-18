use std::iter::once;

use either::Either;
use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::{Body, Local, Place, StatementKind, TerminatorKind},
    ty::TyCtxt,
};

use flowistry::{
    infoflow::{Direction, FlowAnalysis},
    mir::{engine, placeinfo::PlaceInfo},
};

use rustc_utils::mir::location_or_arg::LocationOrArg;

use crate::analysis::scrutinizer::scrutinizer_local::ScrutinizerBody;

// This function computes all locals that depend on the argument local for a given def_id.
pub fn compute_dependent_locals<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    targets: Vec<Vec<(Place<'tcx>, LocationOrArg)>>,
    direction: Direction,
    body_with_facts: ScrutinizerBody<'tcx>,
) -> Vec<Local> {
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

    log::trace!(
        "computing location dependencies for {:?}, {:?}",
        def_id,
        targets
    );

    // Use Flowistry to compute the locations and places influenced by the target.
    let location_deps =
        flowistry::infoflow::compute_dependencies(&results, targets.clone(), direction)
            .into_iter()
            .reduce(|acc, e| {
                let mut new_acc = acc.clone();
                new_acc.union(&e);
                new_acc
            })
            .unwrap();

    log::trace!("location deps: {location_deps:?}");

    // Merge location dependencies and extract locals from them.
    let dependent_locals = location_deps
        .iter()
        .map(|dep| match dep {
            LocationOrArg::Location(location) => {
                let stmt_or_terminator = body.stmt_at(*location);
                match stmt_or_terminator {
                    Either::Left(stmt) => match &stmt.kind {
                        StatementKind::Assign(assign) => {
                            let (place, _) = **assign;
                            vec![place.local]
                        }
                        _ => {
                            unimplemented!()
                        }
                    },
                    Either::Right(terminator) => match &terminator.kind {
                        TerminatorKind::Call {
                            destination, args, ..
                        } => once(destination.local)
                            .chain(
                                args.iter()
                                    .filter_map(|arg| arg.place().map(|place| place.local)),
                            )
                            .collect(),
                        TerminatorKind::SwitchInt { .. } => vec![],
                        _ => {
                            unimplemented!()
                        }
                    },
                }
            }
            LocationOrArg::Arg(local) => vec![*local],
        })
        .flatten()
        .collect();

    drop(body_with_facts);
    drop(body);

    dependent_locals
}
