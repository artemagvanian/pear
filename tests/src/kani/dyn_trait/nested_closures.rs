// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Check that we can codegen various nesting structures of boxes and
// pointer to closures.

#[pear::analysis_entry]
fn main() {
    // Create a nested boxed once-callable closure
    let f: Box<Box<dyn FnOnce(i32)>> = Box::new(Box::new(|x| assert!(x == 1)));
    f(1);

    // Create a pointer to a closure
    let g = |x: f32, y: i32| {
        assert!(x == 1.0);
        assert!(y == 2)
    };
    let p: &dyn Fn(f32, i32) = &g;
    p(1.0, 2);

    // Additional level of pointer nesting
    let g_1 = |x: f32, y: i32| {
        assert!(x == 1.0);
        assert!(y == 2)
    };
    let p_1: &dyn Fn(f32, i32) = &g_1;
    let q: &dyn Fn(f32, i32) = &p_1;
    q(1.0, 2);

    // Create a boxed pointer to a closure
    let r: Box<&dyn Fn(f32, i32, bool)> = Box::new(&|x: f32, y: i32, z: bool| {
        assert!(x == 1.0);
        assert!(y == 2);
        assert!(z);
    });
    r(1.0, 2, true);

    // Another boxed box
    let s: Box<Box<dyn Fn(i32)>> = Box::new(Box::new(|x| assert!(x == 3)));
    s(3);

    // A pointer to the boxed box
    let s_1: Box<Box<dyn Fn(i32)>> = Box::new(Box::new(|x| assert!(x == 3)));
    let t: &Box<Box<dyn Fn(i32)>> = &s_1;
    t(3);
}
