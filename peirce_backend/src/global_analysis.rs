use std::fs;

use regex::Regex;
use rustc_hir::ItemKind;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{self, FnSig, Ty, TyCtxt},
};

use crate::{reachability::collect_mono_items_from, refiner::refine_from};

pub trait GlobalAnalysis<'tcx> {
    fn construct(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation;
}

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
    fn construct(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation {
        tcx.hir().items().for_each(|item_id| {
            let hir = tcx.hir();
            let item = hir.item(item_id);
            let def_id = item.owner_id.to_def_id();
            let def_path_str = tcx.def_path_str(def_id);

            if !self
                .filter
                .as_ref()
                .map(|filter| filter.is_match(def_path_str.as_str()))
                .unwrap_or(true)
            {
                return;
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
                    return;
                }

                let (items, usage_map) =
                    collect_mono_items_from(tcx, MonoItem::Fn(instance), !self.skip_generic);

                fs::write(
                    format!("{def_path_str}.peirce.json"),
                    serde_json::to_string_pretty(&usage_map)
                        .expect("failed to serialize collection results"),
                )
                .expect("failed to write collection results to a file");

                let refined_usage_graph = refine_from(instance, items, tcx);
                fs::write(
                    format!("{def_path_str}.refined.peirce.json"),
                    serde_json::to_string_pretty(&refined_usage_graph)
                        .expect("failed to serialize refinement results"),
                )
                .expect("failed to write refinement results to a file");
            }
        });
        rustc_driver::Compilation::Continue
    }
}
