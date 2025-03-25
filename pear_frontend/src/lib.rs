#![feature(rustc_private, box_patterns, min_specialization)]

extern crate either;
extern crate polonius_engine;
extern crate rustc_ast;
extern crate rustc_borrowck;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_macros;
extern crate rustc_middle;
extern crate rustc_serialize;
extern crate rustc_span;
extern crate rustc_type_ir;

pub mod analysis;
pub mod logging;
pub mod pear_plugin;
pub mod scrutinizer_plugin;
