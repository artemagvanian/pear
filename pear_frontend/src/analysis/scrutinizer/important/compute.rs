use itertools::Itertools;
use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::{Body, Place, Terminator, TerminatorKind},
    ty::TyCtxt,
};

use flowistry::{
    infoflow::{Direction, FlowAnalysis},
    mir::{engine, placeinfo::PlaceInfo},
};

use rustc_utils::{mir::location_or_arg::LocationOrArg, PlaceExt};

use crate::analysis::scrutinizer::{analyzer::ImportantArgs, scrutinizer_local::ScrutinizerBody};

#[derive(Debug)]
pub struct DependentTerminator<'tcx> {
    terminator: Terminator<'tcx>,
    is_implicitly_dependent: bool,
}

impl<'tcx> DependentTerminator<'tcx> {
    pub fn new(terminator: Terminator<'tcx>, is_implicitly_dependent: bool) -> Self {
        Self {
            terminator,
            is_implicitly_dependent,
        }
    }
    pub fn terminator(&self) -> &Terminator<'tcx> {
        &self.terminator
    }

    pub fn is_implicitly_dependent(&self) -> bool {
        self.is_implicitly_dependent
    }
}

// This function computes all locals that depend on the argument local for a given def_id.
pub fn compute_dependent_terminators<'tcx>(
    def_id: DefId,
    important_args: ImportantArgs,
    body_with_facts: ScrutinizerBody<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> Vec<DependentTerminator<'tcx>> {
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
                    // Check if any of the important args flow into the terminator as arguments.
                    let has_explicit_flows_into = {
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
                                match &important_args {
                                    ImportantArgs::Args(important_args) => {
                                        important_args.contains(&local)
                                    }
                                    ImportantArgs::AllImplicitlyImportant => true,
                                }
                            } else {
                                false
                            }
                        })
                    };

                    // Check if any of the important args influence the terminator's execution indirectly.
                    let has_implicit_flows_into = {
                        match &important_args {
                            ImportantArgs::Args(important_args) => {
                                flowistry::infoflow::compute_dependencies(
                                    &results,
                                    vec![important_args
                                        .iter()
                                        .map(|arg| {
                                            (Place::make(*arg, &[], tcx), LocationOrArg::Arg(*arg))
                                        })
                                        .collect()],
                                    Direction::Forward,
                                )[0]
                                .iter()
                                .any(|location_or_arg| {
                                    if let LocationOrArg::Location(location) = *location_or_arg {
                                        terminator_loc == location
                                    } else {
                                        false
                                    }
                                })
                            }
                            // Propagate the implicit taint.
                            ImportantArgs::AllImplicitlyImportant => true,
                        }
                    };

                    (has_implicit_flows_into || has_explicit_flows_into).then_some(
                        DependentTerminator::new(terminator.clone(), has_implicit_flows_into),
                    )
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
                            match &important_args {
                                ImportantArgs::Args(important_args) => {
                                    important_args.contains(&local)
                                }
                                ImportantArgs::AllImplicitlyImportant => true,
                            }
                        } else {
                            false
                        }
                    })
                    .then_some(DependentTerminator {
                        is_implicitly_dependent: false,
                        terminator: terminator.clone(),
                    })
                }
                _ => None,
            }
        })
        .collect();

    drop(body_with_facts);
    drop(body);

    dependent_terminators
}
