use rustc_hir::def_id::LocalDefId;
use rustc_macros::{TyDecodable, TyEncodable};
use rustc_middle::{
    mir::{Body, ClearCrossCrate, StatementKind},
    ty::TyCtxt,
};
use rustc_serialize::{Decodable, Encodable};
use rustc_utils::mir::borrowck_facts::get_body_with_borrowck_facts;

use crate::caching::{PeirceDecoder, PeirceEncoder};

pub trait LocalAnalysis<'tcx> {
    type Out: Encodable<PeirceEncoder<'tcx>> + for<'a> Decodable<PeirceDecoder<'tcx, 'a>>;

    fn construct(&self, tcx: TyCtxt<'tcx>, local_def_id: LocalDefId) -> Self::Out;
}

pub struct CachedBodyAnalysis {}

impl<'tcx> LocalAnalysis<'tcx> for CachedBodyAnalysis {
    type Out = CachedBody<'tcx>;

    fn construct(&self, tcx: TyCtxt<'tcx>, local_def_id: LocalDefId) -> Self::Out {
        CachedBody::retrieve(tcx, local_def_id)
    }
}

/// A mir [`Body`] and all the additional borrow checking facts that our
/// points-to analysis needs.
#[derive(TyDecodable, TyEncodable, Debug)]
pub struct CachedBody<'tcx> {
    body: Body<'tcx>,
}

impl<'tcx> CachedBody<'tcx> {
    pub fn new(body: Body<'tcx>) -> Self {
        Self { body }
    }

    /// Retrieve a body and the necessary facts for a local item.
    ///
    /// Ensure this is called early enough in the compiler so that the body has not been stolen yet.
    pub fn retrieve(tcx: TyCtxt<'tcx>, local_def_id: LocalDefId) -> Self {
        let body_with_facts = get_body_with_borrowck_facts(tcx, local_def_id);
        let mut body = body_with_facts.body.clone();
        clean_undecodable_data_from_body(&mut body);

        Self { body }
    }

    pub fn owned_body(self) -> Body<'tcx> {
        self.body
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
