use std::fs;

use colored::Colorize;
use itertools::Itertools;
use regex::Regex;
use rustc_hir::ItemKind;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{self, FnSig, Ty, TyCtxt},
};
use rustc_span::Symbol;

use pear_backend::{collect_from, refine_from, GlobalAnalysis, RefinedUsageGraph};
use rustc_utils::BodyExt;

use crate::analysis::utils::instance_sig;

pub struct DumpingGlobalAnalysis {
    filter: Option<Regex>,
}

impl<'tcx> DumpingGlobalAnalysis {
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

/// Dumps the usage map from each entry function to a file.
/// Loads MIR [`Body`]s retrieved during LocalAnalysis via call to substituted_mir(). `
impl<'tcx> GlobalAnalysis<'tcx> for DumpingGlobalAnalysis {
    fn perform_analysis(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation {
        colored::control::set_override(true);

        println!("{}", "Starting PEAR analysis.".blue().bold());

        let pear_entry_attribute = [Symbol::intern("pear"), Symbol::intern("analysis_entry")];
        let hir = tcx.hir();

        for item_id in tcx.hir().items() {
            let item = hir.item(item_id);
            let def_id = item.owner_id.to_def_id();
            let def_path_str = tcx.def_path_str(def_id);

            if tcx
                .get_attrs_by_path(def_id, &pear_entry_attribute)
                .next()
                .is_none()
            {
                continue;
            }

            if !self
                .filter
                .as_ref()
                .map(|filter| filter.is_match(def_path_str.as_str()))
                .unwrap_or(true)
            {
                continue;
            }

            if let ItemKind::Fn(..) = &item.kind {
                let instance =
                    ty::Instance::new(def_id, ty::GenericArgs::identity_for_item(tcx, def_id));

                let instance_sig: FnSig = instance_sig(instance, tcx);

                if instance_sig
                    .inputs_and_output
                    .iter()
                    .any(|ty| contains_non_concrete_type(ty))
                {
                    println!("WARNING: the function passed to analysis contains dynamic types; MCG construction might be incomplete.")
                }

                let entry_instance = match tcx.asyncness(def_id) {
                    ty::Asyncness::Yes => {
                        let intermediate_instance = ty::Instance::new(
                            def_id,
                            ty::GenericArgs::identity_for_item(tcx, def_id),
                        );
                        let intermediate_body = tcx.instance_mir(intermediate_instance.def);
                        let inner_coroutine_type = intermediate_body.return_ty();
                        let ty::TyKind::Coroutine(inner_coroutine_def_id, ..) =
                            inner_coroutine_type.kind().clone()
                        else {
                            unreachable!()
                        };
                        ty::Instance::new(
                            inner_coroutine_def_id,
                            ty::GenericArgs::identity_for_item(tcx, inner_coroutine_def_id),
                        )
                    }
                    ty::Asyncness::No => {
                        ty::Instance::new(def_id, ty::GenericArgs::identity_for_item(tcx, def_id))
                    }
                };

                let (items, usage_map) = collect_from(tcx, MonoItem::Fn(entry_instance));

                for item in items.iter() {
                    if let MonoItem::Fn(instance) = item.item()
                        && tcx.is_mir_available(instance.def_id())
                    {
                        let body = tcx.instance_mir(instance.def);
                        fs::create_dir_all("bodies").expect("failed to create bodies dir");
                        fs::write(
                            format!("bodies/{}.mir.rs", tcx.def_path_str(instance.def_id())),
                            body.to_string(tcx).unwrap(),
                        )
                        .expect("failed to write body into a file");
                    }
                }

                let serialized_collection_results = serde_json::to_string_pretty(&usage_map)
                    .expect("failed to serialize collection results");
                fs::write(
                    format!("{def_path_str}.pear.json"),
                    serialized_collection_results,
                )
                .expect("failed to write collection results to a file");

                let refined_usage_graph = refine_from(entry_instance, items, tcx);
                let serialized_refinement_results =
                    serde_json::to_string_pretty(&refined_usage_graph)
                        .expect("failed to serialize refinement results");

                if let Ok(bytes) =
                    fs::read(format!("expected/{def_path_str}.refined.pear.expected"))
                {
                    let expected =
                        String::from_utf8(bytes).expect("failed to parse the expected file");
                    run_test(&def_path_str, &refined_usage_graph, &expected);
                }

                fs::write(
                    format!("{def_path_str}.refined.pear.json"),
                    serialized_refinement_results,
                )
                .expect("failed to write refinement results to a file");
            }
        }
        colored::control::unset_override();
        rustc_driver::Compilation::Continue
    }
}

fn run_test(def_path_str: &str, refined_usage_graph: &RefinedUsageGraph, expected: &str) {
    println!("{}", format!("  [{def_path_str}]").blue().bold(),);
    let instances = refined_usage_graph
        .instances()
        .into_iter()
        .map(|instance| instance.to_string())
        .collect_vec();
    for line in expected.lines() {
        if !instances.contains(&String::from(line)) {
            println!("{}", "    Test failed.".red().bold());
            println!(
                "{}",
                format!("      {line} is not present in the refined graph.").red()
            );
            return;
        }
    }
    println!("{}", "    Test passed.".green());
}
