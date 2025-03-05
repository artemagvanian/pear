// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Check that we can pass a dyn function pointer to a simple closure

fn takes_dyn_fun(fun: &dyn Fn() -> i32) {
    let x = fun();
    assert!(x == 5);
}
#[pear::analysis_entry]
fn main() {
    let closure = || 5;
    takes_dyn_fun(&closure)
}
