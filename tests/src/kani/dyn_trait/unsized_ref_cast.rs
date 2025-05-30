// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Check that we can cast between two unsized objects.
use std::rc::Rc;

trait Byte {
    fn eq(&self, byte: u8) -> bool;
}

impl Byte for u8 {
    fn eq(&self, byte: u8) -> bool {
        *self == byte
    }
}

fn all_zero_ref(num: &dyn Byte) -> bool {
    num.eq(0x0)
}

#[pear::analysis_entry]
fn check_ref() {
    let num: u8 = 42;
    let fat_ptr: &(dyn Byte + Sync) = &num;
    assert!(!all_zero_ref(fat_ptr));
}
