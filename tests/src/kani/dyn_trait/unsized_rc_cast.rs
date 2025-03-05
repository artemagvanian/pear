// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Check that we can cast between two unsized objects.
//! This fix-me is derived from unsized_rc_cast.rs and it should be merged with the original test.
//! The issue https://github.com/model-checking/kani/issues/1528 tracks the fix for this testcase.
use std::rc::Rc;

trait Byte {
    fn eq(&self, byte: u8) -> bool;
}

impl Byte for u8 {
    fn eq(&self, byte: u8) -> bool {
        *self == byte
    }
}

fn all_zero_rc(num: Rc<dyn Byte>) -> bool {
    num.eq(0x0)
}

#[pear::analysis_entry]
fn check_rc() {
    let num: u8 = 42;
    let rc: Rc<dyn Byte + Sync> = Rc::new(num);
    assert!(!all_zero_rc(rc));
}
