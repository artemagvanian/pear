use rustc_middle::ty::{self, FnSig, Instance, InstanceDef, Ty};

pub fn is_virtual<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Virtual(..))
}

pub fn is_intrinsic<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Intrinsic(..))
}

/// Checks if type a is equivalent to type b taking into account subtyping relations.
pub fn ty_eq_with_subtyping<'tcx>(ty_a: Ty<'tcx>, ty_b: Ty<'tcx>) -> bool {
    ty_a.walk().any(|generic_arg| {
        generic_arg
            .as_type()
            .map(|ty| matches!(ty.kind(), ty::Foreign(..)))
            .unwrap_or(false)
    }) || ty_a == ty_b
}

/// Checks if function signature a is equivalent to function signature b taking into account
/// subtyping relations.
pub fn fn_sig_eq_with_subtyping<'tcx>(fn_sig_a: FnSig<'tcx>, fn_sig_b: FnSig<'tcx>) -> bool {
    let ty_eq_with_subtyping = {
        let inputs_and_output_a = fn_sig_a.inputs_and_output;
        let inputs_and_output_b = fn_sig_b.inputs_and_output;
        inputs_and_output_a.len() == inputs_and_output_b.len()
            && inputs_and_output_a
                .iter()
                .zip(inputs_and_output_b.iter())
                .all(|(ty_a, ty_b)| ty_eq_with_subtyping(ty_a, ty_b))
    };
    ty_eq_with_subtyping
        && fn_sig_a.unsafety == fn_sig_b.unsafety
        && fn_sig_a.c_variadic == fn_sig_b.c_variadic
        && fn_sig_a.abi == fn_sig_b.abi
}
