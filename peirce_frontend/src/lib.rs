#![feature(rustc_private, box_patterns, min_specialization)]

extern crate rustc_driver;
extern crate rustc_interface;

use clap::Parser;

use regex::Regex;
use rustc_plugin::{CrateFilter, RustcPlugin, RustcPluginArgs, Utf8Path};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, env, process::Command};

pub mod logging;

pub struct PeircePlugin;

#[derive(Parser, Serialize, Deserialize)]
pub struct PeircePluginArgs {
    #[clap(short, long, default_value = "true")]
    skip_generics: bool,
    #[clap(short, long)]
    filter: Option<String>,
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
        peirce_backend::modify_cargo(cargo);
    }

    fn run(
        self,
        mut compiler_args: Vec<String>,
        plugin_args: Self::Args,
    ) -> rustc_interface::interface::Result<()> {
        peirce_backend::modify_compiler_args(&mut compiler_args);

        let mut callbacks = match peirce_backend::how_to_handle_this_crate(&mut compiler_args) {
            peirce_backend::CrateHandling::Noop => {
                Box::new(peirce_backend::NoopCallbacks) as Box<dyn rustc_driver::Callbacks + Send>
            }
            peirce_backend::CrateHandling::LocalAnalysis => Box::new(
                peirce_backend::LocalAnalysisCallbacks::new(peirce_backend::CachedBodyAnalysis {}),
            ),
            peirce_backend::CrateHandling::GlobalAnalysis => {
                Box::new(peirce_backend::GlobalAnalysisCallbacks::new(
                    peirce_backend::DumpingGlobalAnalysis::new(
                        plugin_args.filter.map(|filter| {
                            Regex::new(filter.as_str()).expect("failed to compile filter regex")
                        }),
                        plugin_args.skip_generics,
                    ),
                    peirce_backend::CachedBodyAnalysis {},
                ))
            }
        };
        rustc_driver::RunCompiler::new(&compiler_args, callbacks.as_mut()).run()
    }
}
