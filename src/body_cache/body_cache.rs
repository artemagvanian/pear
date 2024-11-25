use std::path::PathBuf;

use rustc_hir::{
    def_id::{CrateNum, DefId, LocalDefId, LOCAL_CRATE},
    intravisit::{self},
};
use rustc_macros::{TyDecodable, TyEncodable};
use rustc_middle::{
    hir::nested_filter::OnlyBodies,
    mir::{Body, ClearCrossCrate, StatementKind},
    ty::TyCtxt,
};

use rustc_serialize::{Decodable, Encodable};
use rustc_utils::mir::borrowck_facts::get_body_with_borrowck_facts;

use crate::body_cache::encoder::{decode_from_file, encode_to_file, PeirceDecoder, PeirceEncoder};

/// A mir [`Body`] and all the additional borrow checking facts that our
/// points-to analysis needs.
#[derive(TyDecodable, TyEncodable, Debug)]
pub struct CachedBody<'tcx> {
    body: Body<'tcx>,
}

impl<'tcx> CachedBody<'tcx> {
    /// Retrieve a body and the necessary facts for a local item.
    ///
    /// Ensure this is called early enough in the compiler so that the body has not been stolen yet.
    fn retrieve(tcx: TyCtxt<'tcx>, local_def_id: LocalDefId) -> Self {
        let body_with_facts = get_body_with_borrowck_facts(tcx, local_def_id);
        let mut body = body_with_facts.body.clone();
        clean_undecodable_data_from_body(&mut body);

        Self { body }
    }

    pub fn owned_body(self) -> Body<'tcx> {
        self.body
    }
}

pub trait LocalAnalysis<'tcx> {
    type Out: Encodable<PeirceEncoder<'tcx>> + for<'a> Decodable<PeirceDecoder<'tcx, 'a>>;

    fn construct(
        &self,
        tcx: TyCtxt<'tcx>,
        local_def_id: LocalDefId,
    ) -> Self::Out;
}

pub struct CachedBodyAnalysis {}

impl<'tcx> LocalAnalysis<'tcx> for CachedBodyAnalysis {
    type Out = CachedBody<'tcx>;

    fn construct(
        &self,
        tcx: TyCtxt<'tcx>,
        local_def_id: LocalDefId,
    ) -> Self::Out {
        Self::Out::retrieve(tcx, local_def_id)
    }
}

/// A visitor to collect all bodies in the crate and write them to disk.
struct DumpingVisitor<'tcx, A: LocalAnalysis<'tcx>> {
    tcx: TyCtxt<'tcx>,
    target_dir: PathBuf,
    analysis: A,
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

impl<'tcx, A: LocalAnalysis<'tcx>> intravisit::Visitor<'tcx> for DumpingVisitor<'tcx, A> {
    type NestedFilter = OnlyBodies;
    fn nested_visit_map(&mut self) -> Self::Map {
        self.tcx.hir()
    }

    fn visit_fn(
        &mut self,
        function_kind: intravisit::FnKind<'tcx>,
        function_declaration: &'tcx rustc_hir::FnDecl<'tcx>,
        body_id: rustc_hir::BodyId,
        _: rustc_span::Span,
        local_def_id: LocalDefId,
    ) {
        let to_write = self.analysis.construct(self.tcx, local_def_id);

        let dir = &self.target_dir;
        let path = dir.join(
            self.tcx
                .def_path(local_def_id.to_def_id())
                .to_filename_friendly_no_crate(),
        );

        if !dir.exists() {
            std::fs::create_dir(dir).unwrap();
        }

        encode_to_file(self.tcx, path, &to_write);

        intravisit::walk_fn(
            self,
            function_kind,
            function_declaration,
            body_id,
            local_def_id,
        )
    }
}

/// A complete visit over the local crate items, collecting all bodies and
/// calculating the necessary borrowcheck facts to store for later points-to
/// analysis.
///
/// Ensure this gets called early in the compiler before the unoptimmized mir
/// bodies are stolen.
pub fn dump_mir_and_borrowck_facts<'tcx, A: LocalAnalysis<'tcx>>(tcx: TyCtxt<'tcx>, analysis: A) {
    let mut vis = DumpingVisitor {
        tcx,
        target_dir: intermediate_out_dir(tcx, INTERMEDIATE_ARTIFACT_EXT),
        analysis,
    };
    tcx.hir().visit_all_item_likes_in_crate(&mut vis);
}

const INTERMEDIATE_ARTIFACT_EXT: &str = "peirce_cache";

/// Get the path where artifacts from this crate would be stored. Unlike
/// [`TyCtxt::crate_extern_paths`] this function does not crash when supplied
/// with [`LOCAL_CRATE`].
pub fn local_or_remote_paths(krate: CrateNum, tcx: TyCtxt, ext: &str) -> Vec<PathBuf> {
    if krate == LOCAL_CRATE {
        vec![intermediate_out_dir(tcx, ext)]
    } else {
        tcx.crate_extern_paths(krate)
            .iter()
            .map(|p| p.with_extension(ext))
            .collect()
    }
}

/// Try to load a [`CachedBody`] for this id.
pub fn load_body_and_facts<'tcx, A: LocalAnalysis<'tcx>>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
) -> Result<A::Out, String> {
    let paths = local_or_remote_paths(def_id.krate, tcx, INTERMEDIATE_ARTIFACT_EXT);
    for path in &paths {
        let path = path.join(tcx.def_path(def_id).to_filename_friendly_no_crate());
        if let Ok(data) = decode_from_file(tcx, path) {
            return Ok(data);
        };
    }
    return Err(format!(
        "No facts for {def_id:?} found at any path tried: {paths:?}"
    ));
}

/// Create the name of the file in which to store intermediate artifacts.
///
/// HACK(Justus): `TyCtxt::output_filenames` returns a file stem of
/// `lib<crate_name>-<hash>`, whereas `OutputFiles::with_extension` returns a file
/// stem of `<crate_name>-<hash>`. I haven't found a clean way to get the same
/// name in both places, so i just assume that these two will always have this
/// relation and prepend the `"lib"` here.
pub fn intermediate_out_dir(tcx: TyCtxt, ext: &str) -> PathBuf {
    let rustc_out_file = tcx.output_filenames(()).with_extension(ext);
    let dir = rustc_out_file
        .parent()
        .unwrap_or_else(|| panic!("{} has no parent", rustc_out_file.display()));
    let file = rustc_out_file
        .file_name()
        .unwrap_or_else(|| panic!("has no file name"))
        .to_str()
        .unwrap_or_else(|| panic!("not utf8"));

    let file = if file.starts_with("lib") {
        std::borrow::Cow::Borrowed(file)
    } else {
        format!("lib{file}").into()
    };

    dir.join(file.as_ref())
}
