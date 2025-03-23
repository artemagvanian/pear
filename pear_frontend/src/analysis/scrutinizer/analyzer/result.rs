use rustc_hir::def_id::DefId;
use rustc_middle::ty::Instance;
use serde::{ser::{SerializeStruct, SerializeTuple}, Serialize, Serializer};

pub fn serialize_instance<S>(instance: &Instance, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut tup = serializer.serialize_tuple(2)?;
    tup.serialize_element(instance.to_string().as_str())?;
    tup.serialize_element(format!("{instance:?}").as_str())?;
    tup.end()
}

#[derive(Serialize)]
pub struct FunctionWithMetadata<'tcx> {
    #[serde(serialize_with = "serialize_instance")]
    function: Instance<'tcx>,
    raw_pointer_deref: bool,
    allowlisted: bool,
    has_transmute: bool,
}

impl<'tcx> FunctionWithMetadata<'tcx> {
    pub fn new(
        function: Instance<'tcx>,
        raw_pointer_deref: bool,
        allowlisted: bool,
        has_transmute: bool,
    ) -> Self {
        FunctionWithMetadata {
            function,
            raw_pointer_deref,
            allowlisted,
            has_transmute,
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy)]
pub enum ImpurityReason {
    MutableArguments,
    UnresolvedGenerics,
    ImpureInnerFunction,
}

pub struct PurityAnalysisResult<'tcx> {
    def_id: DefId,
    annotated_pure: bool,
    status: bool,
    reason: Option<ImpurityReason>,
    passing: Vec<FunctionWithMetadata<'tcx>>,
    failing: Vec<FunctionWithMetadata<'tcx>>,
}

impl<'tcx> PurityAnalysisResult<'tcx> {
    pub fn new(
        def_id: DefId,
        annotated_pure: bool,
        status: bool,
        reason: Option<ImpurityReason>,
        passing: Vec<FunctionWithMetadata<'tcx>>,
        failing: Vec<FunctionWithMetadata<'tcx>>,
    ) -> Self {
        Self {
            def_id,
            annotated_pure,
            status,
            reason,
            passing,
            failing,
        }
    }

    pub fn status(&self) -> bool {
        self.status
    }

    pub fn annotated_pure(&self) -> bool {
        self.annotated_pure
    }

    pub fn reason(&self) -> Option<ImpurityReason> {
        self.reason
    }

    pub fn error(def_id: DefId, reason: Option<ImpurityReason>, annotated_pure: bool) -> Self {
        Self::new(def_id, annotated_pure, false, reason, vec![], vec![])
    }
}

impl<'tcx> Serialize for PurityAnalysisResult<'tcx> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("PurityAnalysisResult", 8)?;
        state.serialize_field("def_id", format!("{:?}", self.def_id).as_str())?;
        state.serialize_field("annotated_pure", &self.annotated_pure)?;
        state.serialize_field("status", &self.status)?;
        if !self.status {
            state.serialize_field("reason", &self.reason)?;
        }
        state.serialize_field("passing", &self.passing)?;
        state.serialize_field("failing", &self.failing)?;
        state.end()
    }
}
