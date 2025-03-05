#![feature(rustc_private, box_patterns, min_specialization)]

extern crate rustc_borrowck;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_macros;
extern crate rustc_middle;
extern crate rustc_serialize;
extern crate rustc_span;
extern crate rustc_type_ir;

use clap::Parser;

use regex::Regex;
use rustc_plugin::{CrateFilter, RustcPlugin, RustcPluginArgs, Utf8Path};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, env, process::Command};

pub mod logging;
mod analysis;

pub struct PearPlugin;

#[derive(Parser, Serialize, Deserialize)]
pub struct PearPluginArgs {
    #[clap(short, long, default_value = "true")]
    skip_generics: bool,
    #[clap(short, long)]
    filter: Option<String>,
    #[clap(last = true)]
    cargo_args: Vec<String>,
}

impl RustcPlugin for PearPlugin {
    type Args = PearPluginArgs;

    fn version(&self) -> Cow<'static, str> {
        env!("CARGO_PKG_VERSION").into()
    }

    fn driver_name(&self) -> Cow<'static, str> {
        "pear-driver".into()
    }

    fn args(&self, _target_dir: &Utf8Path) -> RustcPluginArgs<Self::Args> {
        let args = PearPluginArgs::parse_from(env::args().skip(1));
        let filter = CrateFilter::AllCrates;
        RustcPluginArgs { args, filter }
    }

    fn modify_cargo(&self, cargo: &mut Command, _args: &Self::Args) {
        pear_backend::modify_cargo(cargo);
    }

    fn run(
        self,
        mut compiler_args: Vec<String>,
        plugin_args: Self::Args,
    ) -> rustc_interface::interface::Result<()> {
        pear_backend::modify_compiler_args(&mut compiler_args);

        let mut callbacks = match pear_backend::how_to_handle_this_crate(&mut compiler_args) {
            pear_backend::CrateHandling::Noop => {
                Box::new(pear_backend::NoopCallbacks) as Box<dyn rustc_driver::Callbacks + Send>
            }
            pear_backend::CrateHandling::LocalAnalysis => Box::new(
                pear_backend::LocalAnalysisCallbacks::new(analysis::CachedBodyAnalysis {}),
            ),
            pear_backend::CrateHandling::GlobalAnalysis => {
                Box::new(pear_backend::GlobalAnalysisCallbacks::new(
                    analysis::DumpingGlobalAnalysis::new(
                        plugin_args.filter.map(|filter| {
                            Regex::new(filter.as_str()).expect("failed to compile filter regex")
                        }),
                        plugin_args.skip_generics,
                    ),
                    analysis::CachedBodyAnalysis {},
                ))
            }
        };
        rustc_driver::RunCompiler::new(&compiler_args, callbacks.as_mut()).run()
    }
}
