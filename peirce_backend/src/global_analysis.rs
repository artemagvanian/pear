use std::fs;

use rustc_hir::ItemKind;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{self, TyCtxt},
};

use crate::{reachability::collect_mono_items_from, utils::substituted_mir};

pub trait GlobalAnalysis<'tcx> {
    fn construct(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation;
}

pub struct DumpingGlobalAnalysis {}

/// Dumps the usage map from each entry function to a file. 
/// Loads MIR [`Body`]s retrieved during LocalAnalysis via call to substituted_mir(). ` 
impl<'tcx> GlobalAnalysis<'tcx> for DumpingGlobalAnalysis {
    fn construct(&self, tcx: TyCtxt<'tcx>) -> rustc_driver::Compilation {
        tcx.hir().items().for_each(|item_id| {
            let hir = tcx.hir();
            let item = hir.item(item_id);
            let def_id = item.owner_id.to_def_id();

            if let ItemKind::Fn(..) = &item.kind {
                let instance =
                    ty::Instance::new(def_id, ty::GenericArgs::identity_for_item(tcx, def_id));
                let (items, usage_map) = collect_mono_items_from(tcx, MonoItem::Fn(instance));
                for item in items.into_iter() {
                    match item {
                        MonoItem::Fn(instance) => {
                            let _body = substituted_mir(&instance, tcx).unwrap();
                        }
                        MonoItem::Static(_def_id) => {}
                        MonoItem::GlobalAsm(_item_id) => {}
                    }
                }
                fs::write(
                    format!("{:?}.peirce.json", item_id.owner_id),
                    serde_json::to_string_pretty(&usage_map).unwrap(),
                )
                .unwrap();
            }
        });
        rustc_driver::Compilation::Continue
    }
}
