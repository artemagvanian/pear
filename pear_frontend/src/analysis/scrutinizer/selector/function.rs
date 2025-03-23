use rustc_hir::ItemKind;
use rustc_middle::ty::{self, GenericArgs, TyCtxt};
use rustc_span::Symbol;

pub fn select_functions<'tcx>(tcx: TyCtxt<'tcx>) -> Vec<(ty::Instance<'tcx>, bool)> {
    let scrutinizer_pure_attribute = [Symbol::intern("pear"), Symbol::intern("scrutinizer_pure")];

    let scrutinizer_impure_attribute =
        [Symbol::intern("pear"), Symbol::intern("scrutinizer_impure")];

    let hir = tcx.hir();

    tcx.hir()
        .items()
        .filter_map(|item_id| {
            let item = hir.item(item_id);
            let def_id = item.owner_id.to_def_id();

            let annotated_pure;
            if tcx
                .get_attrs_by_path(def_id, &scrutinizer_pure_attribute)
                .next()
                .is_some()
            {
                annotated_pure = true;
            } else if tcx
                .get_attrs_by_path(def_id, &scrutinizer_impure_attribute)
                .next()
                .is_some()
            {
                annotated_pure = false;
            } else {
                return None;
            }

            if let ItemKind::Fn(..) = &item.kind {
                // Retrieve the instance, as we know it exists.
                let args = GenericArgs::identity_for_item(tcx, def_id);
                let instance = ty::Instance::new(def_id, args);
                Some((instance, annotated_pure))
            } else {
                None
            }
        })
        .collect()
}
