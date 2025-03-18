use std::fs;

use itertools::Itertools;
use pear_backend::RefinedUsageGraph;
use regex::Regex;
use rustc_middle::mir::{Mutability, VarDebugInfoContents};
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_span::symbol::Symbol;
use rustc_utils::BodyExt;

use crate::analysis::scrutinizer::scrutinizer_local::{
    substituted_mir, ScrutinizerBody, SubstitutedMirErrorKind,
};
use crate::analysis::scrutinizer::{
    analyzer::{
        heuristics::{HasRawPtrDeref, HasTransmute},
        result::{FunctionWithMetadata, PurityAnalysisResult},
    },
    important::ImportantLocals,
};

use super::result::ImpurityReason;

fn analyze_item<'tcx>(
    item: Instance<'tcx>,
    scrutinizer_body: Option<ScrutinizerBody<'tcx>>,
    important_locals: ImportantLocals,
    passing_calls_ref: &mut Vec<FunctionWithMetadata<'tcx>>,
    failing_calls_ref: &mut Vec<FunctionWithMetadata<'tcx>>,
    storage: &RefinedUsageGraph<'tcx>,
    allowlist: &Vec<Regex>,
    trusted_stdlib: &Vec<Regex>,
    stack: &mut Vec<Instance<'tcx>>,
    tcx: TyCtxt<'tcx>,
) -> bool {
    // Check if allowlisted.
    let is_allowlisted = {
        let def_path_str = format!("{:?}", item.def_id());
        allowlist.iter().any(|lib| lib.is_match(&def_path_str))
    };
    if is_allowlisted {
        let info_with_metadata = FunctionWithMetadata::new(
            item.to_owned(),
            important_locals.clone(),
            false,
            is_allowlisted,
            false,
        );
        passing_calls_ref.push(info_with_metadata);
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
        passing_calls_ref.push(info_with_metadata);
        return true;
    }

    // Check if has no body (i.e. intrinsic or foreign).
    match &scrutinizer_body {
        Some(scrutinizer_body) => dump_body(item, scrutinizer_body.clone(), tcx),
        None => {
            let info_with_metadata = FunctionWithMetadata::new(
                item.to_owned(),
                important_locals.clone(),
                false,
                false,
                false,
            );
            failing_calls_ref.push(info_with_metadata);
            return false;
        }
    }

    // Get optimized MIR for heuristics.
    let optimized_mir = tcx.instance_mir(item.def);

    // Check if conditionally trusted as an std member.
    let is_trusted = {
        let def_path_str = format!("{:?}", item.def_id());
        let trusted_stdlib_member = trusted_stdlib.iter().any(|lib| lib.is_match(&def_path_str));
        let self_ty = {
            optimized_mir
                .var_debug_info
                .iter()
                .find(|dbg_info| dbg_info.name == Symbol::intern("self"))
                .and_then(|self_dbg_info| match self_dbg_info.value {
                    VarDebugInfoContents::Place(place) => Some(place),
                    _ => None,
                })
                .and_then(|self_place| Some(self_place.ty(optimized_mir, tcx).ty))
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
    let has_raw_pointer_deref = optimized_mir.has_raw_ptr_deref(tcx);
    let has_transmute = optimized_mir.has_transmute(tcx);

    // Check if trusted.
    if is_trusted {
        let info_with_metadata = FunctionWithMetadata::new(
            item.to_owned(),
            important_locals.clone(),
            has_raw_pointer_deref,
            is_allowlisted,
            has_transmute,
        );
        passing_calls_ref.push(info_with_metadata);
        true
    } else {
        // Check if has no leaking calls.
        let has_no_leaking_calls = storage
            .get_forward_edges(&item)
            .into_iter()
            .flat_map(|child_node| {
                child_node
                    .instances()
                    .into_iter()
                    .map(|child_item| {
                        if stack.contains(&child_item) {
                            return true;
                        } else {
                            let child_scrutinizer_body = substituted_mir(child_item, tcx);
                            let args = scrutinizer_body
                                .clone()
                                .unwrap()
                                .get_args_by_call_span(child_node.span());
                            let new_important_locals = important_locals.transition(
                                &args,
                                child_item,
                                child_scrutinizer_body.clone().ok(),
                                tcx,
                            );
                            match child_scrutinizer_body {
                                Ok(child_scrutinizer_body) => {
                                    stack.push(child_item);
                                    let result = analyze_item(
                                        child_item,
                                        Some(child_scrutinizer_body),
                                        new_important_locals,
                                        passing_calls_ref,
                                        failing_calls_ref,
                                        storage,
                                        allowlist,
                                        trusted_stdlib,
                                        stack,
                                        tcx,
                                    );
                                    stack.pop();
                                    result
                                }
                                Err(err_kind) => match err_kind {
                                    SubstitutedMirErrorKind::UnimportantMir => true,
                                    SubstitutedMirErrorKind::NoCallableMir
                                    | SubstitutedMirErrorKind::NoMirFound => {
                                        stack.push(child_item);
                                        let result = analyze_item(
                                            child_item,
                                            None,
                                            new_important_locals,
                                            passing_calls_ref,
                                            failing_calls_ref,
                                            storage,
                                            allowlist,
                                            trusted_stdlib,
                                            stack,
                                            tcx,
                                        );
                                        stack.pop();
                                        result
                                    }
                                },
                            }
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
            passing_calls_ref.push(info_with_metadata);
            true
        } else {
            let info_with_metadata = FunctionWithMetadata::new(
                item.to_owned(),
                important_locals.clone(),
                has_raw_pointer_deref,
                is_allowlisted,
                has_transmute,
            );
            failing_calls_ref.push(info_with_metadata);
            false
        }
    }
}

pub fn run<'tcx>(
    functions: RefinedUsageGraph<'tcx>,
    important_locals: ImportantLocals,
    annotated_pure: bool,
    allowlist: &Vec<Regex>,
    trusted_stdlib: &Vec<Regex>,
    tcx: TyCtxt<'tcx>,
) -> PurityAnalysisResult<'tcx> {
    let origin = functions.root();

    let mut passing_calls = vec![];
    let mut failing_calls = vec![];
    let mut stack = vec![origin];

    let pure = analyze_item(
        origin,
        substituted_mir(origin, tcx).ok(),
        important_locals,
        &mut passing_calls,
        &mut failing_calls,
        &functions,
        allowlist,
        trusted_stdlib,
        &mut stack,
        tcx,
    );

    if pure {
        PurityAnalysisResult::new(
            origin.def_id(),
            annotated_pure,
            true,
            None,
            passing_calls,
            failing_calls,
        )
    } else {
        PurityAnalysisResult::new(
            origin.def_id(),
            annotated_pure,
            false,
            Some(ImpurityReason::ImpureInnerFunction),
            passing_calls,
            failing_calls,
        )
    }
}

fn dump_body<'tcx>(
    item: Instance<'tcx>,
    scrutinizer_body: ScrutinizerBody<'tcx>,
    tcx: TyCtxt<'tcx>,
) {
    let (body, _) = scrutinizer_body.split();
    fs::write(
        format!("bodies/{}.mir.rs", tcx.def_path_str(item.def_id())),
        body.to_string(tcx).unwrap(),
    )
    .expect("failed to write body into a file");
}
