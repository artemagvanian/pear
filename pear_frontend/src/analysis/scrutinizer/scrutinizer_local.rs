use flowistry::mir::FlowistryInput;
use itertools::Itertools;
use polonius_engine::FactTypes;
use rustc_borrowck::consumers::RustcFacts;
use rustc_hir::{def::DefKind, def_id::LocalDefId};
use rustc_macros::{Decodable, Encodable, TyDecodable, TyEncodable};
use rustc_middle::{
    mir::{Body, ClearCrossCrate, Operand, StatementKind, TerminatorKind},
    ty::{self, Instance, TyCtxt},
};
use rustc_span::Span;
use rustc_utils::mir::borrowck_facts::get_body_with_borrowck_facts;

use pear_backend::LocalAnalysis;

pub struct ScrutinizerLocalAnalysis {}

impl<'tcx> LocalAnalysis<'tcx> for ScrutinizerLocalAnalysis {
    type Output = ScrutinizerBody<'tcx>;

    fn perform_analysis(&self, tcx: TyCtxt<'tcx>, local_def_id: LocalDefId) -> Self::Output {
        ScrutinizerBody::retrieve(tcx, local_def_id)
    }
}

/// The subset of borrowcheck facts that the points-to analysis (flowistry)
/// needs.
#[derive(Debug, Encodable, Decodable, Clone)]
pub struct FlowistryFacts {
    pub subset_base: Vec<(
        <RustcFacts as FactTypes>::Origin,
        <RustcFacts as FactTypes>::Origin,
    )>,
}

/// A mir [`Body`] and all the additional borrow checking facts that our
/// points-to analysis needs.
#[derive(TyDecodable, TyEncodable, Debug, Clone)]
pub struct ScrutinizerBody<'tcx> {
    body: Body<'tcx>,
    input_facts: FlowistryFacts,
}

impl<'tcx> ScrutinizerBody<'tcx> {
    /// Retrieve a body and the necessary facts for a local item.
    ///
    /// Ensure this is called early enough in the compiler so that the body has not been stolen yet.
    pub fn retrieve(tcx: TyCtxt<'tcx>, local_def_id: LocalDefId) -> Self {
        let body_with_facts = get_body_with_borrowck_facts(tcx, local_def_id);

        let mut body = body_with_facts.body.clone();
        Self::clean_undecodable_data_from_body(&mut body);

        let input_facts = body_with_facts.input_facts.clone();
        let subset_base = input_facts
            .as_ref()
            .unwrap()
            .subset_base
            .iter()
            .map(|&(r1, r2, _)| (r1.into(), r2.into()))
            .collect_vec();

        Self {
            body: body,
            input_facts: FlowistryFacts { subset_base },
        }
    }

    /// Some data in a [Body] is not cross-crate compatible. Usually because it
    /// involves storing a [LocalDefId]. This function makes sure to sanitize those
    /// out.
    fn clean_undecodable_data_from_body(body: &mut Body) {
        for scope in body.source_scopes.iter_mut() {
            scope.local_data = ClearCrossCrate::Clear;
        }

        for stmt in body
            .basic_blocks_mut()
            .iter_mut()
            .flat_map(|bb| bb.statements.iter_mut())
        {
            if matches!(stmt.kind, StatementKind::FakeRead(_)) {
                stmt.make_nop()
            }
        }
    }

    pub fn split(self) -> (Body<'tcx>, FlowistryFacts) {
        (self.body, self.input_facts)
    }

    pub fn map_body(self, f: impl FnOnce(Body<'tcx>) -> Body<'tcx>) -> Self {
        Self {
            body: f(self.body),
            input_facts: self.input_facts,
        }
    }

    pub fn get_args_by_call_span(&self, needle: Span) -> Vec<Vec<Operand<'tcx>>> {
        self.body
            .basic_blocks
            .iter()
            .filter_map(|bb| match &bb.terminator().kind {
                TerminatorKind::Call { fn_span, args, .. } => {
                    (needle.source_equal(*fn_span)).then_some(args)
                }
                _ => None,
            })
            .cloned()
            .collect()
    }
}

impl<'tcx> FlowistryInput<'tcx, 'tcx> for &'tcx ScrutinizerBody<'tcx> {
    fn body(self) -> &'tcx Body<'tcx> {
        &self.body
    }

    fn input_facts_subset_base(
        self,
    ) -> Box<
        (dyn std::iter::Iterator<
            Item = (
                <RustcFacts as FactTypes>::Origin,
                <RustcFacts as FactTypes>::Origin,
            ),
        > + 'tcx),
    > {
        Box::new(self.input_facts.subset_base.iter().copied())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SubstitutedMirErrorKind {
    NoCallableMir,
    UnimportantMir,
    NoMirFound,
}

pub fn substituted_mir<'tcx>(
    instance: Instance<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> Result<ScrutinizerBody<'tcx>, SubstitutedMirErrorKind> {
    let scrutinizer_body = match instance.def {
        ty::InstanceDef::Item(def) => {
            let def_kind = tcx.def_kind(def);
            match def_kind {
                DefKind::Const
                | DefKind::Static(..)
                | DefKind::AssocConst
                | DefKind::Ctor(..)
                | DefKind::AnonConst
                | DefKind::InlineConst => return Err(SubstitutedMirErrorKind::UnimportantMir),
                _ => {
                    let def_id = instance.def_id();
                    match <ScrutinizerLocalAnalysis as LocalAnalysis>::load_local_analysis_results(
                        tcx, def_id,
                    ) {
                        Ok(scrutinizer_body) => scrutinizer_body,
                        Err(_) => return Err(SubstitutedMirErrorKind::NoMirFound),
                    }
                }
            }
        }
        ty::InstanceDef::Virtual(..) | ty::InstanceDef::Intrinsic(..) => {
            return Err(SubstitutedMirErrorKind::NoCallableMir);
        }
        ty::InstanceDef::VTableShim(..)
        | ty::InstanceDef::ReifyShim(..)
        | ty::InstanceDef::FnPtrShim(..)
        | ty::InstanceDef::ClosureOnceShim { .. }
        | ty::InstanceDef::DropGlue(..)
        | ty::InstanceDef::CloneShim(..)
        | ty::InstanceDef::ThreadLocalShim(..)
        | ty::InstanceDef::FnPtrAddrShim(..) => {
            return Err(SubstitutedMirErrorKind::UnimportantMir)
        }
    };
    Ok(scrutinizer_body)
}
