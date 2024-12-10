pub mod leaky_flows {
    pub fn implicit_leak(sensitive_arg: i32) {
        let mut variable = 1;
        // Implicit flow.
        if sensitive_arg > 0 {
            variable = 2;
        }
        println!("{}", variable);
    }

    pub fn reassignment_leak(sensitive_arg: i32) {
        let mut variable = sensitive_arg;
        // Implicit flow.
        if variable > 0 {
            variable = 2;
        }
        println!("{}", variable);
    }
}

pub mod arc_leak {
    use std::sync::{Arc, Mutex};

    pub fn arc_leak(sensitive_arg: i32) {
        let sensitive_arc = Arc::new(Mutex::new(sensitive_arg));
        let sensitive_arc_copy = sensitive_arc.clone();
        let unwrapped = *sensitive_arc_copy.lock().unwrap();
        println!("{}", unwrapped);
    }
}

pub mod tricky_flows {
    pub fn implicit_leak(sensitive_arg: i32) {
        let mut variable = 1;
        // Implicit flow.
        if variable > 0 {
            variable = 2;
        }
        println!("{}", variable);
        if sensitive_arg > 0 {
            variable = 2;
        }
        // This call needs to be revisited.
        println!("{}", variable);
    }
}

pub mod non_leaky_flows {
    pub fn foo(sensitive_arg: i32) {
        let mut variable = 1;
        // Implicit flow.
        if variable > 0 {
            variable = 2;
        }
        println!("{}", variable);
    }
}
