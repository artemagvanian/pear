#![feature(rustc_private, box_patterns, min_specialization)]

#[macro_use]
extern crate tracing;
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

rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

use body_cache::{dump_mir_and_borrowck_facts, substituted_mir};
use clap::Parser;
use reachability::collect_mono_items_from;
use rustc_hir::ItemKind;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{self, TyCtxt},
};
use rustc_plugin::{CrateFilter, RustcPlugin, RustcPluginArgs, Utf8Path};
use rustc_utils::mir::borrowck_facts;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, env, fs, process::Command};

mod body_cache;
mod reachability;

pub struct PeircePlugin;

#[derive(Parser, Serialize, Deserialize)]
pub struct PeircePluginArgs {
    #[clap(last = true)]
    cargo_args: Vec<String>,
}

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

impl RustcPlugin for PeircePlugin {
    type Args = PeircePluginArgs;

    fn version(&self) -> Cow<'static, str> {
        env!("CARGO_PKG_VERSION").into()
    }

    fn driver_name(&self) -> Cow<'static, str> {
        "peirce-driver".into()
    }

    fn args(&self, _target_dir: &Utf8Path) -> RustcPluginArgs<Self::Args> {
        let args = PeircePluginArgs::parse_from(env::args().skip(1));
        let filter = CrateFilter::AllCrates;
        RustcPluginArgs { args, filter }
    }

    fn modify_cargo(&self, cargo: &mut Command, _args: &Self::Args) {
        const CARGO_BUILD_STD_ARG: &str = "-Zbuild-std=std,core,alloc,proc_macro";

        cargo.arg(CARGO_BUILD_STD_ARG);
        cargo.arg(format!("--target={}", get_default_rustc_target().unwrap()));
    }

    fn run(
        self,
        mut compiler_args: Vec<String>,
        plugin_args: Self::Args,
    ) -> rustc_interface::interface::Result<()> {
        const RUSTC_ALWAYS_ENCODE_MIR_ARG: &str = "-Zalways-encode-mir";

        compiler_args.push(RUSTC_ALWAYS_ENCODE_MIR_ARG.into());
        let mut callbacks = match how_to_handle_this_crate(&mut compiler_args) {
            CrateHandling::JustCompile => {
                Box::new(NoopCallbacks) as Box<dyn rustc_driver::Callbacks + Send>
            }
            CrateHandling::CompileAndDump => Box::new(DumpOnlyCallbacks),
            CrateHandling::Analyze => Box::new(PeircePluginCallbacks { args: plugin_args }),
        };
        rustc_driver::RunCompiler::new(&compiler_args, callbacks.as_mut()).run()
    }
}

enum CrateHandling {
    JustCompile,
    CompileAndDump,
    Analyze,
}

fn how_to_handle_this_crate(compiler_args: &mut Vec<String>) -> CrateHandling {
    const BUILD_SCRIPT_CRATE_NAME: &str = "build_script_build";
    const CARGO_PRIMARY_PACKAGE_ENV_VAR: &str = "CARGO_PRIMARY_PACKAGE";

    let crate_name = compiler_args
        .iter()
        .enumerate()
        .find_map(|(i, s)| (s == "--crate-name").then_some(i))
        .and_then(|i| compiler_args.get(i + 1))
        .cloned();

    match &crate_name {
        Some(krate) if krate == BUILD_SCRIPT_CRATE_NAME => CrateHandling::JustCompile,
        _ if std::env::var(CARGO_PRIMARY_PACKAGE_ENV_VAR).is_ok() => CrateHandling::Analyze,
        Some(_) => CrateHandling::CompileAndDump,
        _ => CrateHandling::JustCompile,
    }
}

struct NoopCallbacks;

impl rustc_driver::Callbacks for NoopCallbacks {}

struct DumpOnlyCallbacks;

impl rustc_driver::Callbacks for DumpOnlyCallbacks {
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
            dump_mir_and_borrowck_facts(tcx);
        });
        rustc_driver::Compilation::Continue
    }
}

struct PeircePluginCallbacks {
    args: PeircePluginArgs,
}

impl rustc_driver::Callbacks for PeircePluginCallbacks {
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
            dump_mir_and_borrowck_facts(tcx);
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
            .enter(|tcx| dump_all_items(tcx, &self.args))
    }
}

// Analysis callback.
fn dump_all_items(tcx: TyCtxt, _args: &PeircePluginArgs) -> rustc_driver::Compilation {
    tcx.hir().items().for_each(|item_id| {
        let hir = tcx.hir();
        let item = hir.item(item_id);
        let def_id = item.owner_id.to_def_id();

        if let ItemKind::Fn(..) = &item.kind {
            let instance =
                ty::Instance::new(def_id, ty::GenericArgs::identity_for_item(tcx, def_id));
            let (items, usage_map) = collect_mono_items_from(tcx, MonoItem::Fn(instance));
            for item in items.into_iter() {
                match item {
                    MonoItem::Fn(instance) => {
                        let _body = substituted_mir(&instance, tcx).unwrap();
                    }
                    MonoItem::Static(_def_id) => {}
                    MonoItem::GlobalAsm(_item_id) => {}
                }
            }
            fs::write(
                format!("{:?}.peirce.json", item_id.owner_id),
                serde_json::to_string_pretty(&usage_map).unwrap(),
            )
            .unwrap();
        }
    });
    rustc_driver::Compilation::Continue
}
