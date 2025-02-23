// Copyright Kani Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Check that we can cast &&Box<dyn Error + Send + Sync> to &dyn Debug
// without panicking

use std::error::Error;
use std::fmt::{Debug, Display, Formatter, Result};

#[derive(Debug)]
struct Concrete;
impl Error for Concrete {}

impl Display for Concrete {
    fn fmt(&self, f: &mut Formatter) -> Result {
        Ok(())
    }
}

fn f<'a>(x: &'a &Box<dyn Error + Send + Sync>) -> Box<&'a dyn Display> {
    let d = x as &dyn Display;
    Box::new(d)
}

#[pear::analysis_entry]
fn main() {
    let c = Concrete {};
    let x = Box::new(c) as Box<dyn Error + Send + Sync>;
    let r = &x;
    let d = f(&r);
    let fmt = format!("{}", d);
}
