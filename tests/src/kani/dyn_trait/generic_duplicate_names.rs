// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// This test checks that we can handle potential naming conflicts when
// generic types give the same Trait::function name pairs.

trait Foo<T> {
    fn method(&self, t: T) -> T;
}

trait Bar: Foo<u32> + Foo<i32> {}

impl<T> Foo<T> for () {
    fn method(&self, t: T) -> T {
        t
    }
}

impl Bar for () {}

#[pear::analysis_entry]
fn main() {
    let b: &dyn Bar = &();
    // The vtable for b will now have two Foo::method entries,
    // one for Foo<u32> and one for Foo<i32>.
    let result = <dyn Bar as Foo<u32>>::method(b, 22_u32);
    assert!(result == 22_u32);
}
