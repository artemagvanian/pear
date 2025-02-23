// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Test that we can handle passing closure as function pointer.

/// Invoke given function with the given 'input'.
fn invoke(input: usize, f: fn(usize) -> usize) -> usize {
    f(input)
}

#[pear::analysis_entry]
fn check_closure_ptr() {
    let input = 42;
    let output = invoke(input, |x| x);
    assert_eq!(output, input);
}
