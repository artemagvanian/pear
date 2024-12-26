use rustc_middle::ty::{self, FnSig, Instance, InstanceDef, Ty};

pub fn is_virtual<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Virtual(..))
}

pub fn is_intrinsic<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Intrinsic(..))
}

/// Returns true if the type contains an inner type that is not concrete enough for the refinement
/// purposes (e.g., a type parameter, a function pointer, or a dynamic type).
pub fn instantiation_of<'tcx>(generic: Ty<'tcx>, particular: Ty<'tcx>) -> bool {
    let mut generic_walker = generic.walk();
    let mut particular_walker = particular.walk();
    loop {
        let next_generic = generic_walker.next();
        let next_particular = particular_walker.next();

        match (next_generic, next_particular) {
            (Some(generic_arg), Some(particular_arg)) => {
                let generic_arg = generic_arg.unpack();
                let particular_arg = particular_arg.unpack();

                match (generic_arg, particular_arg) {
                    (
                        ty::GenericArgKind::Type(generic_ty),
                        ty::GenericArgKind::Type(particular_ty),
                    ) => {
                        if matches!(
                            generic_ty.kind(),
                            ty::Param(..)
                                | ty::Foreign(..)
                                | ty::Dynamic(..)
                                | ty::Alias(..)
                                | ty::FnPtr(..)
                        ) || matches!(
                            particular_ty.kind(),
                            ty::Param(..)
                                | ty::Foreign(..)
                                | ty::Dynamic(..)
                                | ty::Alias(..)
                                | ty::FnPtr(..)
                        ) {
                            generic_walker.skip_current_subtree();
                            particular_walker.skip_current_subtree();
                        } else {
                            match (generic_ty.kind(), particular_ty.kind()) {
                                (ty::Bool, ty::Bool)
                                | (ty::Char, ty::Char)
                                | (ty::Int(..), ty::Int(..))
                                | (ty::Uint(..), ty::Uint(..))
                                | (ty::Float(..), ty::Float(..))
                                | (ty::Str, ty::Str)
                                | (ty::Infer(..), ty::Infer(..))
                                | (ty::Bound(.., _), ty::Bound(.., _))
                                | (ty::Placeholder(_), ty::Placeholder(_))
                                | (ty::Error(_), ty::Error(_))
                                | (ty::Never, ty::Never) => {
                                    if generic_ty != particular_ty {
                                        return false;
                                    }
                                }
                                (ty::Tuple(_), ty::Tuple(_))
                                | (ty::Slice(_), ty::Slice(_))
                                | (ty::Array(_, _), ty::Array(_, _)) => {}
                                (ty::RawPtr(tm_generic), ty::RawPtr(tm_particular)) => {
                                    if tm_generic.mutbl != tm_particular.mutbl {
                                        return false;
                                    }
                                }
                                (
                                    ty::Ref(_, _, mutability_generic),
                                    ty::Ref(_, _, mutability_particular),
                                ) => {
                                    if mutability_generic != mutability_particular {
                                        return false;
                                    }
                                }
                                (ty::Adt(adt_def_generic, _), ty::Adt(adt_def_particular, _)) => {
                                    if adt_def_generic != adt_def_particular {
                                        return false;
                                    }
                                }
                                (ty::FnDef(def_id_generic, _), ty::FnDef(def_id_particular, _))
                                | (
                                    ty::Closure(def_id_generic, _),
                                    ty::Closure(def_id_particular, _),
                                )
                                | (
                                    ty::Coroutine(def_id_generic, _),
                                    ty::Coroutine(def_id_particular, _),
                                )
                                | (
                                    ty::CoroutineWitness(def_id_generic, _),
                                    ty::CoroutineWitness(def_id_particular, _),
                                ) => {
                                    if def_id_generic != def_id_particular {
                                        return false;
                                    }
                                }
                                _ => return false,
                            }
                        }
                    }
                    (
                        ty::GenericArgKind::Lifetime(generic_region),
                        ty::GenericArgKind::Lifetime(particular_region),
                    ) => {
                        assert!(generic_region.is_erased() && particular_region.is_erased());
                    }
                    (
                        ty::GenericArgKind::Const(generic_const),
                        ty::GenericArgKind::Const(particular_const),
                    ) => {
                        if generic_const != particular_const {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
            (None, None) => {
                return true;
            }
            _ => return false,
        }
    }
}

/// Returns true if `particular` function signature is a potential resolution of `generic` function
/// signature. In other words, it checks if the concrete (non-generic like) parameters of the
/// function signatures match.
pub fn is_resolution_of<'tcx>(generic: FnSig<'tcx>, particular: FnSig<'tcx>) -> bool {
    // Check that the number of inputs matches to use `zip` later.
    if generic.inputs().len() != particular.inputs().len() {
        return false;
    }

    generic
        .inputs_and_output
        .iter()
        .zip(particular.inputs_and_output.iter())
        .all(|(superset_ty, subset_ty)| instantiation_of(superset_ty, subset_ty))
}
