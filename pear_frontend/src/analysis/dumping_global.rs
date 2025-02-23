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

pub struct DumpingGlobalAnalysis {
    filter: Option<Regex>,
    skip_generic: bool,
}

impl<'tcx> DumpingGlobalAnalysis {
    pub fn new(filter: Option<Regex>, skip_generic: bool) -> Self {
        Self {
            filter,
            skip_generic,
        }
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

                let instance_sig: FnSig = tcx.instantiate_bound_regions_with_erased(
                    tcx.erase_regions(
                        tcx.fn_sig(instance.def_id())
                            .instantiate(tcx, instance.args),
                    ),
                );

                if self.skip_generic
                    && instance_sig
                        .inputs_and_output
                        .iter()
                        .any(|ty| contains_non_concrete_type(ty))
                {
                    continue;
                }

                let (items, usage_map) =
                    collect_from(tcx, MonoItem::Fn(instance), !self.skip_generic);

                let serialized_collection_results = serde_json::to_string_pretty(&usage_map)
                    .expect("failed to serialize collection results");
                fs::write(
                    format!("{def_path_str}.pear.json"),
                    serialized_collection_results,
                )
                .expect("failed to write collection results to a file");

                let refined_usage_graph = refine_from(instance, items, tcx);
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
        rustc_driver::Compilation::Continue
    }
}

fn run_test(def_path_str: &str, refined_usage_graph: &RefinedUsageGraph, expected: &str) {
    colored::control::set_override(true);
    println!(
        "{}",
        format!("Running refinement test: {def_path_str}...")
            .blue()
            .bold(),
    );
    let mut instances = refined_usage_graph
        .instances()
        .into_iter()
        .map(|instance| instance.to_string());
    for line in expected.lines() {
        if !instances.contains(line) {
            println!(
                "{}",
                format!("Refinement test {def_path_str} failed.")
                    .red()
                    .bold()
            );
            return;
        }
    }
    println!(
        "{}",
        format!("Refinement test {def_path_str} passed.").green()
    );
    colored::control::unset_override( );
}
