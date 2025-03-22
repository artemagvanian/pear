use std::fs;

use itertools::Itertools;
use pear_backend::{RefinedNode, RefinedUsageGraph};
use regex::Regex;
use rustc_middle::mir::{Body, Local, Mutability, Operand, TerminatorKind, VarDebugInfoContents};
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_span::symbol::Symbol;
use rustc_span::Span;
use rustc_utils::BodyExt;

use crate::analysis::scrutinizer::scrutinizer_local::{substituted_mir, SubstitutedMirErrorKind};
use crate::analysis::scrutinizer::{
    analyzer::{
        heuristics::{HasRawPtrDeref, HasTransmute},
        result::{FunctionWithMetadata, PurityAnalysisResult},
    },
    important::ImportantLocals,
};

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
        body: Option<Body<'tcx>>,
        important_locals: ImportantLocals,
    ) -> bool {
        // Check if allowlisted.
        let is_allowlisted = {
            let def_path_str = format!("{:?}", item.def_id());
            self.allowlist.iter().any(|lib| lib.is_match(&def_path_str))
        };
        if is_allowlisted {
            let info_with_metadata = FunctionWithMetadata::new(
                item.to_owned(),
                important_locals.clone(),
                false,
                is_allowlisted,
                false,
            );
            self.passing_calls.push(info_with_metadata);
            return true;
        }

        // Check if has no important calls.
        let has_no_important_locals = important_locals.is_empty();
        if has_no_important_locals {
            let info_with_metadata = FunctionWithMetadata::new(
                item.to_owned(),
                important_locals.clone(),
                false,
                false,
                false,
            );
            self.passing_calls.push(info_with_metadata);
            return true;
        }

        // Check if has no body (i.e. intrinsic or foreign).
        let body = match &body {
            Some(body) => {
                dump_body(item, body.clone(), self.tcx);
                body
            }
            None => {
                let info_with_metadata = FunctionWithMetadata::new(
                    item.to_owned(),
                    important_locals.clone(),
                    false,
                    false,
                    false,
                );
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
                important_locals.clone(),
                has_raw_pointer_deref,
                is_allowlisted,
                has_transmute,
            );
            self.passing_calls.push(info_with_metadata);
            true
        } else {
            // Check if has no leaking calls.
            let has_no_leaking_calls = self
                .storage
                .get_forward_edges(&item)
                .into_iter()
                .flat_map(|child_node| {
                    child_node
                        .instances()
                        .into_iter()
                        .map(|child_item| {
                            if self.stack.contains(&child_item) {
                                return true;
                            } else {
                                self.analyze_child(
                                    body.clone(),
                                    important_locals.clone(),
                                    child_item,
                                    child_node.clone(),
                                )
                            }
                        })
                        .collect_vec()
                })
                .all(|r| r);

            if !has_raw_pointer_deref && !has_transmute && has_no_leaking_calls {
                let info_with_metadata = FunctionWithMetadata::new(
                    item.to_owned(),
                    important_locals.clone(),
                    has_raw_pointer_deref,
                    is_allowlisted,
                    has_transmute,
                );
                self.passing_calls.push(info_with_metadata);
                true
            } else {
                let info_with_metadata = FunctionWithMetadata::new(
                    item.to_owned(),
                    important_locals.clone(),
                    has_raw_pointer_deref,
                    is_allowlisted,
                    has_transmute,
                );
                self.failing_calls.push(info_with_metadata);
                false
            }
        }
    }

    fn analyze_child(
        &mut self,
        parent_body: Body<'tcx>,
        parent_important_locals: ImportantLocals,
        child_item: Instance<'tcx>,
        child_node: RefinedNode<'tcx>,
    ) -> bool {
        let child_scrutinizer_body = substituted_mir(child_item, self.tcx);
        let args = get_args_by_call_span(&parent_body, child_node.span());
        args.into_iter().all(|args| {
            let new_important_locals = parent_important_locals.transition(
                &args,
                child_item,
                child_scrutinizer_body.clone().ok(),
                self.tcx,
            );
            match child_scrutinizer_body.clone() {
                Ok(child_scrutinizer_body) => {
                    let (child_body, _) = child_scrutinizer_body.split();
                    self.stack.push(child_item);
                    let result =
                        self.analyze_item(child_item, Some(child_body), new_important_locals);
                    self.stack.pop();
                    result
                }
                Err(err_kind) => match err_kind {
                    SubstitutedMirErrorKind::UnimportantMir => {
                        // Skip analyzing the unimportant mir, check children directly.
                        let parent_body = self.tcx.instance_mir(child_item.def).clone();
                        let parent_important_locals = ImportantLocals::new(
                            (0..parent_body.local_decls.len())
                                .map(|idx| Local::from_usize(idx))
                                .collect(),
                        );

                        self.stack.push(child_item);
                        let result = self.storage
                            .get_forward_edges(&child_item)
                            .into_iter()
                            .flat_map(|child_node| {
                                child_node
                                    .instances()
                                    .into_iter()
                                    .map(|child_item| {
                                        if self.stack.contains(&child_item) {
                                            return true;
                                        } else {
                                            self.analyze_child(
                                                parent_body.clone(),
                                                parent_important_locals.clone(),
                                                child_item,
                                                child_node.clone(),
                                            )
                                        }
                                    })
                                    .collect_vec()
                            })
                            .all(|r| r);
                        self.stack.pop();
                        result
                    }
                    SubstitutedMirErrorKind::NoCallableMir
                    | SubstitutedMirErrorKind::NoMirFound => {
                        self.stack.push(child_item);
                        let result = self.analyze_item(child_item, None, new_important_locals);
                        self.stack.pop();
                        result
                    }
                },
            }
        })
    }

    pub fn run(
        functions: RefinedUsageGraph<'tcx>,
        important_locals: ImportantLocals,
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

        let body = substituted_mir(origin, tcx)
            .ok()
            .map(|scrutinizer_body| scrutinizer_body.split().0);

        let pure = analysis.analyze_item(origin, body, important_locals);

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

fn dump_body<'tcx>(item: Instance<'tcx>, body: Body<'tcx>, tcx: TyCtxt<'tcx>) {
    fs::create_dir_all("bodies").expect("failed to create bodies dir");
    fs::write(
        format!("bodies/{}.mir.rs", tcx.def_path_str(item.def_id())),
        body.to_string(tcx).unwrap(),
    )
    .expect("failed to write body into a file");
}

fn get_args_by_call_span<'tcx>(body: &Body<'tcx>, span: Span) -> Vec<Vec<Operand<'tcx>>> {
    body.basic_blocks
        .iter()
        .filter_map(|bb| match &bb.terminator().kind {
            TerminatorKind::Call { fn_span, args, .. } => {
                (!span.is_dummy() && span.source_equal(*fn_span)).then_some(args.clone())
            }
            TerminatorKind::Drop { place, .. } => {
                (span.is_dummy()).then_some(vec![Operand::Copy(*place)])
            }
            _ => None,
        })
        .collect()
}
