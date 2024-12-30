use rustc_middle::ty::{Instance, InstanceDef};

pub fn is_virtual<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Virtual(..))
}

pub fn is_intrinsic<'tcx>(instance: Instance<'tcx>) -> bool {
    matches!(instance.def, InstanceDef::Intrinsic(..))
}
