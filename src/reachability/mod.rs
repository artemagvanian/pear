use rustc_hir::lang_items::LangItem;
use rustc_middle::query::TyCtxtAt;
use rustc_middle::traits;
use rustc_middle::ty::adjustment::CustomCoerceUnsized;
use rustc_middle::ty::{self, Ty};

mod collector;
mod errors;

pub use collector::collect_mono_items_from;

fn custom_coerce_unsize_info<'tcx>(
    tcx: TyCtxtAt<'tcx>,
    source_ty: Ty<'tcx>,
    target_ty: Ty<'tcx>,
) -> CustomCoerceUnsized {
    let trait_ref = ty::TraitRef::from_lang_item(
        tcx.tcx,
        LangItem::CoerceUnsized,
        tcx.span,
        [source_ty, target_ty],
    );

    match tcx.codegen_select_candidate((ty::ParamEnv::reveal_all(), trait_ref)) {
        Ok(traits::ImplSource::UserDefined(traits::ImplSourceUserDefinedData {
            impl_def_id,
            ..
        })) => tcx.coerce_unsized_info(impl_def_id).custom_kind.unwrap(),
        impl_source => {
            panic!("invalid `CoerceUnsized` impl_source: {:?}", impl_source);
        }
    }
}
