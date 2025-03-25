#![feature(rustc_private, box_patterns, min_specialization, let_chains)]

#[macro_use]
extern crate rustc_middle;
extern crate polonius_engine;
extern crate rustc_borrowck;
extern crate rustc_const_eval;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_fluent_macro;
extern crate rustc_hash;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_macros;
extern crate rustc_monomorphize;
extern crate rustc_serialize;
extern crate rustc_session;
extern crate rustc_smir;
extern crate rustc_span;
extern crate rustc_target;
extern crate rustc_type_ir;

use rustc_utils::mir::borrowck_facts;
use std::process::Command;

mod analysis;
mod caching;
mod reachability;
mod refiner;
mod serialize;
mod utils;

pub use analysis::global_analysis::GlobalAnalysis;
pub use analysis::local_analysis::LocalAnalysis;
pub use reachability::{collect_from, Node, Usage, UsageGraph};
pub use refiner::{refine_from, RefinedNode, RefinedUsageGraph, TransitiveRefinedNode};

fn get_default_rustc_target() -> Result<String, String> {
    const RUSTC_COMMAND: &str = "rustc";
    const HOST_PREFIX: &str = "host: ";
    const VERBOSE_VERSION_ARG: &str = "-vV";

    let output = Command::new(RUSTC_COMMAND)
        .arg(VERBOSE_VERSION_ARG)
        .output()
        .map_err(|_| "cannot get default rustc target")?;

    let stdout = String::from_utf8(output.stdout).map_err(|_| "cannot parse stdout")?;

    let mut target = String::from("");
    for part in stdout.split("\n") {
        if part.starts_with(HOST_PREFIX) {
            target = part.chars().skip(HOST_PREFIX.len()).collect();
        }
    }

    if target.len() == 0 {
        return Err("cannot find target".into());
    }

    Ok(target)
}

pub fn modify_cargo(cargo: &mut Command) {
    const CARGO_BUILD_STD_ARG: &str = "-Zbuild-std=std,core,alloc,proc_macro";
    cargo.arg(CARGO_BUILD_STD_ARG);
    cargo.arg(format!("--target={}", get_default_rustc_target().unwrap()));
}

pub fn modify_compiler_args(compiler_args: &mut Vec<String>) {
    const RUSTC_ALWAYS_ENCODE_MIR_ARG: &str = "-Zalways-encode-mir";
    const RUSTC_REGISTER_TOOL: &str = "-Zcrate-attr=feature(register_tool)";
    const RUSTC_REGISTER_PEAR: &str = "-Zcrate-attr=register_tool(pear)";
    compiler_args.extend([
        RUSTC_ALWAYS_ENCODE_MIR_ARG.into(),
        RUSTC_REGISTER_TOOL.into(),
        RUSTC_REGISTER_PEAR.into(),
    ]);
}

pub enum CrateHandling {
    Noop,
    LocalAnalysis,
    GlobalAnalysis,
}

pub fn how_to_handle_this_crate(compiler_args: &mut Vec<String>) -> CrateHandling {
    const BUILD_SCRIPT_CRATE_NAME: &str = "build_script_build";
    const CARGO_PRIMARY_PACKAGE_ENV_VAR: &str = "CARGO_PRIMARY_PACKAGE";

    let crate_name = compiler_args
        .iter()
        .enumerate()
        .find_map(|(i, s)| (s == "--crate-name").then_some(i))
        .and_then(|i| compiler_args.get(i + 1))
        .cloned();

    match &crate_name {
        Some(krate) if krate == BUILD_SCRIPT_CRATE_NAME => CrateHandling::Noop,
        _ if std::env::var(CARGO_PRIMARY_PACKAGE_ENV_VAR).is_ok() => CrateHandling::GlobalAnalysis,
        Some(_) => CrateHandling::LocalAnalysis,
        _ => CrateHandling::Noop,
    }
}

pub struct NoopCallbacks;

impl rustc_driver::Callbacks for NoopCallbacks {}

pub struct LocalAnalysisCallbacks<A: for<'a> LocalAnalysis<'a>> {
    local_analysis: A,
}

impl<A: for<'a> LocalAnalysis<'a>> LocalAnalysisCallbacks<A> {
    pub fn new(local_analysis: A) -> Self {
        Self { local_analysis }
    }
}

impl<A: for<'a> LocalAnalysis<'a>> rustc_driver::Callbacks for LocalAnalysisCallbacks<A> {
    fn config(&mut self, config: &mut rustc_interface::Config) {
        // Configure rustc to ensure `get_body_with_borrowck_facts` will work.
        borrowck_facts::enable_mir_simplification();
        config.override_queries = Some(borrowck_facts::override_queries);
    }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> rustc_driver::Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            self.local_analysis.dump_local_analysis_results(tcx);
        });
        rustc_driver::Compilation::Continue
    }
}

pub struct GlobalAnalysisCallbacks<G: for<'a> GlobalAnalysis<'a>, A: for<'a> LocalAnalysis<'a>> {
    global_analysis: G,
    local_analysis: A,
}

impl<G: for<'a> GlobalAnalysis<'a>, A: for<'a> LocalAnalysis<'a>> GlobalAnalysisCallbacks<G, A> {
    pub fn new(global_analysis: G, local_analysis: A) -> Self {
        Self {
            global_analysis,
            local_analysis,
        }
    }
}

impl<G: for<'a> GlobalAnalysis<'a>, A: for<'a> LocalAnalysis<'a>> rustc_driver::Callbacks
    for GlobalAnalysisCallbacks<G, A>
{
    fn config(&mut self, config: &mut rustc_interface::Config) {
        // Configure rustc to ensure `get_body_with_borrowck_facts` will work.
        borrowck_facts::enable_mir_simplification();
        config.override_queries = Some(borrowck_facts::override_queries);
    }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> rustc_driver::Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            self.local_analysis.dump_local_analysis_results(tcx);
        });
        rustc_driver::Compilation::Continue
    }

    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> rustc_driver::Compilation {
        queries
            .global_ctxt()
            .unwrap()
            .enter(|tcx| self.global_analysis.perform_analysis(tcx))
    }
}
