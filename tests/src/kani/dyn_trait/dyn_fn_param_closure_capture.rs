// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Check that we can pass a dyn function pointer to a closure that captures
// some data

fn takes_dyn_fun(fun: &dyn Fn() -> i32) {
    let x = fun();
    assert!(x == 5);
}

#[pear::analysis_entry]
fn main() {
    let a = vec![3];
    let closure = || a[0] + 2;
    takes_dyn_fun(&closure)
}
