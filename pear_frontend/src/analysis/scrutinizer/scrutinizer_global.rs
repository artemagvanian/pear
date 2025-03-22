use std::fs;

use colored::Colorize;
use regex::Regex;
use rustc_ast::Mutability;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{self, FnSig, Ty, TyCtxt},
};

use pear_backend::{collect_from, refine_from, GlobalAnalysis};
use serde::{Deserialize, Serialize};

use crate::analysis::scrutinizer::{
    analyzer::{ImpurityReason, PurityAnalysisResult, ScrutinizerAnalysis},
    important,
    scrutinizer_local::substituted_mir,
    selector::{select_functions, select_pprs},
    utils::instance_sig,
};

pub struct ScrutinizerGlobalAnalysis {
    filter: Option<Regex>,
}

impl<'tcx> ScrutinizerGlobalAnalysis {
    pub fn new(filter: Option<Regex>) -> Self {
        Self { filter }
    }
}

/// Returns true if the type contains an inner type that is not concrete enough for the refinement
/// purposes (e.g., a type parameter, a function pointer, or a dynamic type).
pub fn contains_non_concrete_type<'tcx>(ty: Ty<'tcx>) -> bool {
    ty.walk().any(|ty| {
        ty.as_type().is_some_and(|ty| {
            matches!(
                ty.kind(),
                ty::Param(..) | ty::FnPtr(..) | ty::Dynamic(..) | ty::Foreign(..)
            )
        })
    })
}

pub fn is_mutable_ref<'tcx>(ty: Ty<'tcx>) -> bool {
    if let ty::TyKind::Ref(.., mutbl) = ty.kind() {
        return mutbl.to_owned() == Mutability::Mut;
    } else {
        return false;
    }
}

fn default_mode() -> String {
    "functions".to_string()
}

fn default_only_inconsistent() -> bool {
    false
}

fn default_output_file() -> String {
    "analysis.result.json".to_string()
}

fn default_shallow() -> bool {
    false
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ScrutinizerConfig {
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default = "default_only_inconsistent")]
    only_inconsistent: bool,
    #[serde(default = "default_output_file")]
    output_file: String,
    #[serde(default = "default_shallow")]
    shallow: bool,

    target_filter: Option<String>,
    important_args: Option<Vec<usize>>,
    allowlist: Option<Vec<String>>,
    trusted_stdlib: Option<Vec<String>>,
}

/// Dumps the usage map from each entry function to a file.
/// Loads MIR [`Body`]s retrieved during LocalAnalysis via call to substituted_mir(). `
impl<'tcx> GlobalAnalysis<'tcx> for ScrutinizerGlobalAnalysis {
    fn perform_analysis(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation {
        colored::control::set_override(true);

        println!("{}", "Starting PEAR-Scrutinizer analysis.".blue().bold());

        let config: ScrutinizerConfig = fs::read("scrutinizer-config.toml")
            .map(|config_bytes| {
                String::from_utf8(config_bytes).expect("failed to parse the expected file")
            })
            .map(|config_str| toml::from_str(&config_str).expect("failed to parse TOML config"))
            .expect("failed to read config file");

        let analysis_targets = if config.mode == "function" {
            select_functions(tcx)
        } else if config.mode == "ppr" {
            select_pprs(tcx)
        } else {
            panic!("unknown mode");
        };

        for (analysis_target, annotated_pure) in analysis_targets {
            let def_id = analysis_target.def_id();
            let def_path_str = tcx.def_path_str(def_id);

            if !self
                .filter
                .as_ref()
                .map(|filter| filter.is_match(def_path_str.as_str()))
                .unwrap_or(true)
            {
                continue;
            }

            let instance_sig: FnSig = instance_sig(analysis_target, tcx);

            let purity_analysis_result = if instance_sig
                .inputs_and_output
                .iter()
                .any(|ty| contains_non_concrete_type(ty))
            {
                PurityAnalysisResult::error(
                    def_id,
                    Some(ImpurityReason::UnresolvedGenerics),
                    annotated_pure,
                )
            } else if instance_sig.inputs().iter().any(|ty| is_mutable_ref(*ty)) {
                PurityAnalysisResult::error(
                    def_id,
                    Some(ImpurityReason::MutableArguments),
                    annotated_pure,
                )
            } else {
                let (items, _) = collect_from(tcx, MonoItem::Fn(analysis_target), false);

                let refined_usage_graph = refine_from(analysis_target, items, tcx);

                // Calculate important locals.
                let important_locals = {
                    let body_with_facts = substituted_mir(analysis_target, tcx)
                        .expect("root object does not have a scrutinizer body");
                    let (body, _) = body_with_facts.clone().split();
                    // Parse important arguments.
                    let important_args = if config.important_args.is_none() {
                        // If no important arguments are provided, assume all are important.
                        let arg_count = { body.arg_count };
                        (1..=arg_count).collect()
                    } else {
                        config.important_args.as_ref().unwrap().to_owned()
                    };
                    important::ImportantLocals::from_important_args(
                        important_args,
                        def_id,
                        body_with_facts,
                        tcx,
                    )
                };

                let allowlist = config
                    .allowlist
                    .as_ref()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|re| Regex::new(re).unwrap())
                    .collect();

                let trusted_stdlib = config
                    .trusted_stdlib
                    .as_ref()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|re| Regex::new(re).unwrap())
                    .collect();

                ScrutinizerAnalysis::run(
                    refined_usage_graph,
                    important_locals,
                    annotated_pure,
                    allowlist,
                    trusted_stdlib,
                    tcx,
                )
            };

            if purity_analysis_result.status() != purity_analysis_result.annotated_pure() {
                let stencil = format!(
                    "{def_path_str} failed; status = {} but annotation = {}; reason = {:?}",
                    purity_analysis_result.status(),
                    purity_analysis_result.annotated_pure(),
                    purity_analysis_result.reason()
                );

                println!(
                    "{}",
                    match purity_analysis_result.annotated_pure() {
                        true => stencil.yellow().bold(),
                        false => stencil.red().bold(),
                    }
                );
            } else {
                println!(
                    "{}",
                    format!(
                        "{def_path_str} passed; status = {} and annotation = {}",
                        purity_analysis_result.status(),
                        purity_analysis_result.annotated_pure()
                    )
                    .green()
                    .bold()
                );
            }

            let serialized_purity_analysis_result =
                serde_json::to_string_pretty(&purity_analysis_result)
                    .expect("failed to serialize purity analysis results");

            fs::write(
                format!("{def_path_str}.purity.pear.json"),
                serialized_purity_analysis_result,
            )
            .expect("failed to write refinement results to a file");
        }
        colored::control::unset_override();
        rustc_driver::Compilation::Continue
    }
}
