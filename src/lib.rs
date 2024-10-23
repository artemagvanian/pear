#![feature(rustc_private)]

#[macro_use]
extern crate tracing;
#[macro_use]
extern crate rustc_middle;

extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_fluent_macro;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lint;
extern crate rustc_macros;
extern crate rustc_monomorphize;
extern crate rustc_session;
extern crate rustc_smir;
extern crate rustc_span;
extern crate rustc_target;

rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

use clap::Parser;
use reachability::collect_mono_items_from;
use rustc_hir::ItemKind;
use rustc_middle::{
    mir::mono::MonoItem,
    ty::{self, TyCtxt},
};
use rustc_plugin::{CrateFilter, RustcPlugin, RustcPluginArgs, Utf8Path};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, env, fs, process::Command};

mod reachability;

pub struct PeircePlugin;

#[derive(Parser, Serialize, Deserialize)]
pub struct PeircePluginArgs {
    #[clap(last = true)]
    cargo_args: Vec<String>,
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
        // Find the default target triplet.
        let output = Command::new("rustc")
            .arg("-vV")
            .output()
            .expect("Cannot get default rustc target");
        let stdout = String::from_utf8(output.stdout).expect("Cannot parse stdout");

        let mut target = String::from("");
        for part in stdout.split("\n") {
            if part.starts_with("host: ") {
                target = part.chars().skip("host: ".len()).collect();
            }
        }
        if target.len() == 0 {
            panic!("Bad output");
        }

        // Add -Zalways-encode-mir to RUSTFLAGS.
        let mut old_rustflags = String::from("");
        for (key, val) in cargo.get_envs() {
            if key == "RUSTFLAGS" {
                if let Some(val) = val {
                    old_rustflags = format!("{}", val.to_str().unwrap());
                }
            }
        }
        cargo.env(
            "RUSTFLAGS",
            format!("-Zalways-encode-mir {}", old_rustflags),
        );
        cargo.arg("-Zbuild-std=std,core,alloc,proc_macro");
        cargo.arg(format!("--target={}", target));
    }

    fn run(
        self,
        compiler_args: Vec<String>,
        plugin_args: Self::Args,
    ) -> rustc_interface::interface::Result<()> {
        let mut callbacks = PeircePluginCallbacks { args: plugin_args };
        let compiler = rustc_driver::RunCompiler::new(&compiler_args, &mut callbacks);
        compiler.run()
    }
}

struct PeircePluginCallbacks {
    args: PeircePluginArgs,
}

impl rustc_driver::Callbacks for PeircePluginCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> rustc_driver::Compilation {
        queries
            .global_ctxt()
            .unwrap()
            .enter(|tcx| print_all_items(tcx, &self.args))
    }
}

// Analysis callback.
fn print_all_items(tcx: TyCtxt, _args: &PeircePluginArgs) -> rustc_driver::Compilation {
    tcx.hir().items().for_each(|item_id| {
        let hir = tcx.hir();
        let item = hir.item(item_id);
        let def_id = item.owner_id.to_def_id();
        if let ItemKind::Fn(..) = &item.kind {
            let instance =
                ty::Instance::new(def_id, ty::GenericArgs::identity_for_item(tcx, def_id));
            fs::write(
                format!("{:?}.peirce.json", item_id.owner_id),
                serde_json::to_string_pretty(
                    &collect_mono_items_from(tcx, MonoItem::Fn(instance)).1,
                )
                .unwrap(),
            )
            .unwrap();
        }
    });
    rustc_driver::Compilation::Continue
}
