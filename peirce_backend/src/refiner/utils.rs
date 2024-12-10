use rustc_hir::Unsafety;
use rustc_middle::ty::{self, FnSig, Instance, InstanceDef, Ty, TyCtxt, TyKind};
use rustc_target::spec::abi::Abi;

pub fn is_virtual<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Virtual(..))
}

pub fn is_intrinsic<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Intrinsic(..))
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

/// Normalizes a function signature for a callable type.
pub fn normalized_fn_sig<'tcx>(ty: Ty<'tcx>, tcx: TyCtxt<'tcx>) -> FnSig<'tcx> {
    let normalized_sig = match ty.kind() {
        TyKind::FnDef(def_id, args) => tcx.fn_sig(*def_id).instantiate(tcx, args),
        TyKind::FnPtr(f) => *f,
        TyKind::Closure(.., args) => {
            tcx.signature_unclosure(args.as_closure().sig(), Unsafety::Normal)
        }
        _ => panic!("unexpected callee type encountered when performing refinement"),
    };
    tcx.instantiate_bound_regions_with_erased(tcx.erase_regions(normalized_sig))
}

/// Returns true if `particular` function signature is a potential resolution of `generic` function
/// signature. In other words, it checks if the concrete (non-generic like) parameters of the
/// function signatures match.
pub fn is_resolution_of<'tcx>(
    generic: FnSig<'tcx>,
    particular: FnSig<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> bool {
    // Normalize the generic function if it is a closure shim.
    let generic_normalized = match generic.abi {
        Abi::Rust | Abi::C { .. } => generic,
        Abi::RustCall => {
            // This is hacky, but we know by construction that the function arguments would be
            // passed as a second argument in a tupled form. For instance, see the following link:
            // https://doc.rust-lang.org/std/ops/trait.Fn.html
            let params = match generic.inputs()[1].kind() {
                ty::Tuple(params) => *params,
                _ => bug!(
                    "encountered a non-tuple as a second argument to a function with Abi::RustCall"
                ),
            };
            tcx.mk_fn_sig(
                params,
                generic.output(),
                generic.c_variadic,
                Unsafety::Normal,
                Abi::Rust,
            )
        }
        _ => bug!("unsupported ABI for refinement: {:?}", generic.abi),
    };

    // Check that the number of inputs matches to use `zip` later.
    if generic_normalized.inputs().len() != particular.inputs().len() {
        return false;
    }

    generic_normalized
        .inputs_and_output
        .iter()
        .zip(particular.inputs_and_output.iter())
        .all(|(superset_ty, subset_ty)| {
            contains_non_concrete_type(superset_ty)
                || contains_non_concrete_type(subset_ty)
                || superset_ty == subset_ty
        })
}

/// Resolves the type of the instance and substitutes the arguments for it.
pub fn instance_type<'tcx>(instance: &Instance<'tcx>, tcx: TyCtxt<'tcx>) -> Ty<'tcx> {
    tcx.type_of(instance.def_id())
        .instantiate(tcx, instance.args)
}
