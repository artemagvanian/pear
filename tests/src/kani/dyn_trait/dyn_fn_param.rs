// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Check that we can pass a dyn function pointer to a stand alone
// function definition

#![feature(ptr_metadata)]

fn takes_dyn_fun(fun: &dyn Fn() -> u32) {
    let x = fun();
    assert!(x == 5);
}

pub fn unit_to_u32() -> u32 {
    5 as u32
}

#[pear::analysis_entry]
fn main() {
    takes_dyn_fun(&unit_to_u32)
}
