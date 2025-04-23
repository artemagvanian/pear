use std::fs;

use itertools::Itertools;
use pear_backend::RefinedUsageGraph;
use regex::Regex;
use rustc_middle::mir::{Local, Mutability, VarDebugInfoContents};
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_span::symbol::Symbol;
use rustc_utils::BodyExt;

use crate::analysis::scrutinizer::analyzer::{
    heuristics::{HasRawPtrDeref, HasTransmute},
    result::{FunctionWithMetadata, PurityAnalysisResult},
};
use crate::analysis::scrutinizer::important::compute_dependent_terminators;
use crate::analysis::scrutinizer::scrutinizer_local::{
    substituted_mir, ScrutinizerBody, SubstitutedMirErrorKind,
};
use crate::analysis::utils::num_args_for_instance;

use super::result::ImpurityReason;

pub struct ScrutinizerAnalysis<'tcx> {
    passing_calls: Vec<FunctionWithMetadata<'tcx>>,
    failing_calls: Vec<FunctionWithMetadata<'tcx>>,
    storage: RefinedUsageGraph<'tcx>,
    allowlist: Vec<Regex>,
    trusted_stdlib: Vec<Regex>,
    stack: Vec<Instance<'tcx>>,
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> ScrutinizerAnalysis<'tcx> {
    fn analyze_item(
        &mut self,
        item: Instance<'tcx>,
        maybe_body_with_facts: Option<ScrutinizerBody<'tcx>>,
        important_args: Vec<Local>,
    ) -> bool {
        // Check if allowlisted.
        let is_allowlisted = {
            let def_path_str = format!("{:?}", item.def_id());
            self.allowlist.iter().any(|lib| lib.is_match(&def_path_str))
        };
        if is_allowlisted {
            let info_with_metadata =
                FunctionWithMetadata::new(item.to_owned(), false, is_allowlisted, false);
            self.passing_calls.push(info_with_metadata);
            return true;
        }

        // Check if has no body (i.e. intrinsic or foreign).
        let body_with_facts = match maybe_body_with_facts {
            Some(body) => {
                dump_body(item, body.clone(), self.tcx);
                body
            }
            None => {
                let info_with_metadata =
                    FunctionWithMetadata::new(item.to_owned(), false, false, false);
                self.failing_calls.push(info_with_metadata);
                return false;
            }
        };

        // Get optimized MIR for heuristics.
        let optimized_mir = self.tcx.instance_mir(item.def);

        // Check if conditionally trusted as an std member.
        let is_trusted = {
            let def_path_str = format!("{:?}", item.def_id());
            let trusted_stdlib_member = self
                .trusted_stdlib
                .iter()
                .any(|lib| lib.is_match(&def_path_str));
            let self_ty = {
                optimized_mir
                    .var_debug_info
                    .iter()
                    .find(|dbg_info| dbg_info.name == Symbol::intern("self"))
                    .and_then(|self_dbg_info| match self_dbg_info.value {
                        VarDebugInfoContents::Place(place) => Some(place),
                        _ => None,
                    })
                    .and_then(|self_place| Some(self_place.ty(optimized_mir, self.tcx).ty))
            };
            let has_immut_self_ref = self_ty
                .and_then(|self_ty| {
                    Some(
                        self_ty
                            .ref_mutability()
                            .and_then(|mutability| Some(mutability == Mutability::Not))
                            .unwrap_or(false),
                    )
                })
                .unwrap_or(false);
            trusted_stdlib_member && !has_immut_self_ref
        };

        // Compute raw pointer dereference and transmute heuristics.
        let has_raw_pointer_deref = optimized_mir.has_raw_ptr_deref(self.tcx);
        let has_transmute = optimized_mir.has_transmute(self.tcx);

        // Check if trusted.
        if is_trusted {
            let info_with_metadata = FunctionWithMetadata::new(
                item.to_owned(),
                has_raw_pointer_deref,
                is_allowlisted,
                has_transmute,
            );
            self.passing_calls.push(info_with_metadata);
            true
        } else {
            if has_raw_pointer_deref || has_transmute {
                let info_with_metadata = FunctionWithMetadata::new(
                    item.to_owned(),
                    has_raw_pointer_deref,
                    is_allowlisted,
                    has_transmute,
                );
                self.failing_calls.push(info_with_metadata);
                return false;
            }

            let important_terminators = compute_dependent_terminators(
                item.def_id(),
                important_args.clone(),
                body_with_facts,
                self.tcx,
            );

            log::debug!(
                "computed important terminators for {} from important args {:?} = {:?}",
                item.to_string(),
                important_args,
                important_terminators
            );

            // Check if has no leaking calls.
            let has_no_leaking_calls =
                self.storage
                    .get_forward_edges(&item)
                    .into_iter()
                    .all(|child_node| {
                        let important_child_node = important_terminators.iter().any(|terminator| {
                            log::debug!(
                                "comparing {:?} with {:?}",
                                terminator.source_info.span,
                                child_node.terminator_span()
                            );
                            terminator
                                .source_info
                                .span
                                .source_equal(child_node.terminator_span())
                        });

                        if important_child_node {
                            child_node.instances().into_iter().all(|child_item| {
                                if self.stack.contains(&child_item) {
                                    return true;
                                } else {
                                    self.analyze_child(child_item)
                                }
                            })
                        } else {
                            true
                        }
                    });

            if has_no_leaking_calls {
                let info_with_metadata = FunctionWithMetadata::new(
                    item.to_owned(),
                    has_raw_pointer_deref,
                    is_allowlisted,
                    has_transmute,
                );
                self.passing_calls.push(info_with_metadata);
                true
            } else {
                let info_with_metadata = FunctionWithMetadata::new(
                    item.to_owned(),
                    has_raw_pointer_deref,
                    is_allowlisted,
                    has_transmute,
                );
                self.failing_calls.push(info_with_metadata);
                false
            }
        }
    }

    fn analyze_child(&mut self, instance: Instance<'tcx>) -> bool {
        let maybe_body_with_facts = substituted_mir(instance, self.tcx);
        let important_args = (1..=num_args_for_instance(instance, self.tcx))
            .map(|arg_num| Local::from_usize(arg_num))
            .collect_vec();

        match maybe_body_with_facts.clone() {
            Ok(body_with_facts) => {
                self.stack.push(instance);
                let result = self.analyze_item(instance, Some(body_with_facts), important_args);
                self.stack.pop();
                result
            }
            Err(err_kind) => match err_kind {
                SubstitutedMirErrorKind::UnimportantMir => {
                    // Skip analyzing the unimportant mir, check children directly.
                    self.stack.push(instance);
                    let result = self
                        .storage
                        .get_forward_edges(&instance)
                        .into_iter()
                        .flat_map(|child_node| {
                            child_node
                                .instances()
                                .into_iter()
                                .map(|child_item| {
                                    if self.stack.contains(&child_item) {
                                        return true;
                                    } else {
                                        self.analyze_child(child_item)
                                    }
                                })
                                .collect_vec()
                        })
                        .all(|r| r);
                    self.stack.pop();
                    result
                }
                SubstitutedMirErrorKind::NoCallableMir | SubstitutedMirErrorKind::NoMirFound => {
                    self.stack.push(instance);
                    let result = self.analyze_item(instance, None, important_args);
                    self.stack.pop();
                    result
                }
            },
        }
    }

    pub fn run(
        functions: RefinedUsageGraph<'tcx>,
        important_args: Vec<Local>,
        annotated_pure: bool,
        allowlist: Vec<Regex>,
        trusted_stdlib: Vec<Regex>,
        tcx: TyCtxt<'tcx>,
    ) -> PurityAnalysisResult<'tcx> {
        let origin = functions.root();

        let mut analysis = Self {
            passing_calls: vec![],
            failing_calls: vec![],
            storage: functions,
            allowlist,
            trusted_stdlib,
            stack: vec![origin],
            tcx,
        };

        let body = substituted_mir(origin, tcx).ok();
        let pure = analysis.analyze_item(origin, body, important_args);

        if pure {
            PurityAnalysisResult::new(
                origin.def_id(),
                annotated_pure,
                true,
                None,
                analysis.passing_calls,
                analysis.failing_calls,
            )
        } else {
            PurityAnalysisResult::new(
                origin.def_id(),
                annotated_pure,
                false,
                Some(ImpurityReason::ImpureInnerFunction),
                analysis.passing_calls,
                analysis.failing_calls,
            )
        }
    }
}

fn dump_body<'tcx>(item: Instance<'tcx>, body: ScrutinizerBody<'tcx>, tcx: TyCtxt<'tcx>) {
    let body = body.split().0;
    fs::create_dir_all("bodies").expect("failed to create bodies dir");
    fs::write(
        format!("bodies/{}.mir.rs", tcx.def_path_str(item.def_id())),
        body.to_string(tcx).unwrap(),
    )
    .expect("failed to write body into a file");
}
