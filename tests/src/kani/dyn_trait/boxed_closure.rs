// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Check that we can codegen a boxed dyn closure

#[pear::analysis_entry]
fn main() {
    // Create a boxed once-callable closure
    let f: Box<dyn FnOnce(f32, i32)> = Box::new(|x, y| {
        assert!(x == 1.0);
        assert!(y == 2);
    });

    // Call it
    f(1.0, 2);
}
