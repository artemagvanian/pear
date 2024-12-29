use rustc_middle::ty::{self, Instance, InstanceDef, Ty};

pub fn is_virtual<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Virtual(..))
}

pub fn is_intrinsic<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Intrinsic(..))
}

/// Returns true if the type contains an inner type that is not concrete enough for the refinement
/// purposes (e.g., a type parameter, a function pointer, or a dynamic type).
pub fn is_instantiation_of<'tcx>(generic: Ty<'tcx>, concrete: Ty<'tcx>) -> bool {
    let mut generic_walker = generic.walk();
    let mut concrete_walker = concrete.walk();
    loop {
        let next_generic = generic_walker.next();
        let next_concrete = concrete_walker.next();

        match (next_generic, next_concrete) {
            (Some(generic_arg), Some(concrete_arg)) => {
                let generic_arg = generic_arg.unpack();
                let concrete_arg = concrete_arg.unpack();

                match (generic_arg, concrete_arg) {
                    (
                        ty::GenericArgKind::Type(generic_ty),
                        ty::GenericArgKind::Type(concrete_ty),
                    ) => {
                        if matches!(
                            generic_ty.kind(),
                            ty::Param(..) | ty::Dynamic(..) | ty::Alias(..) | ty::FnPtr(..)
                        ) || matches!(
                            concrete_ty.kind(),
                            ty::Param(..) | ty::Dynamic(..) | ty::Alias(..) | ty::FnPtr(..)
                        ) {
                            generic_walker.skip_current_subtree();
                            concrete_walker.skip_current_subtree();
                        } else {
                            match (generic_ty.kind(), concrete_ty.kind()) {
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
                                    if generic_ty != concrete_ty {
                                        return false;
                                    }
                                }
                                (ty::Tuple(_), ty::Tuple(_))
                                | (ty::Slice(_), ty::Slice(_))
                                | (ty::Array(_, _), ty::Array(_, _)) => {}
                                (ty::RawPtr(tm_generic), ty::RawPtr(tm_concrete)) => {
                                    if tm_generic.mutbl != tm_concrete.mutbl {
                                        return false;
                                    }
                                }
                                (
                                    ty::Ref(_, _, mutability_generic),
                                    ty::Ref(_, _, mutability_concrete),
                                ) => {
                                    if mutability_generic != mutability_concrete {
                                        return false;
                                    }
                                }
                                (ty::Adt(adt_def_generic, _), ty::Adt(adt_def_concrete, _)) => {
                                    if adt_def_generic != adt_def_concrete {
                                        return false;
                                    }
                                }
                                (ty::FnDef(def_id_generic, _), ty::FnDef(def_id_concrete, _))
                                | (
                                    ty::Closure(def_id_generic, _),
                                    ty::Closure(def_id_concrete, _),
                                )
                                | (
                                    ty::Coroutine(def_id_generic, _),
                                    ty::Coroutine(def_id_concrete, _),
                                )
                                | (
                                    ty::CoroutineWitness(def_id_generic, _),
                                    ty::CoroutineWitness(def_id_concrete, _),
                                ) => {
                                    if def_id_generic != def_id_concrete {
                                        return false;
                                    }
                                }
                                _ => return false,
                            }
                        }
                    }
                    (
                        ty::GenericArgKind::Lifetime(generic_region),
                        ty::GenericArgKind::Lifetime(concrete_region),
                    ) => {
                        assert!(generic_region.is_erased() && concrete_region.is_erased());
                    }
                    (
                        ty::GenericArgKind::Const(generic_const),
                        ty::GenericArgKind::Const(concrete_const),
                    ) => {
                        if generic_const != concrete_const {
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
