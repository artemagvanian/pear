mod leaky_flows {
    #[pear::scrutinizer_impure]
    pub fn implicit_leak(sensitive_arg: i32) {
        let mut variable = 1;
        // Implicit flow.
        if sensitive_arg > 0 {
            variable = 2;
        }
        println!("{}", variable);
    }

    #[pear::scrutinizer_impure]
    pub fn reassignment_leak(sensitive_arg: i32) {
        let mut variable = sensitive_arg;
        // Implicit flow.
        if variable > 0 {
            variable = 2;
        }
        println!("{}", variable);
    }
}

mod arc_leak {
    use std::sync::{Arc, Mutex};

    #[pear::scrutinizer_impure]
    pub fn arc_leak(sensitive_arg: i32) {
        let sensitive_arc = Arc::new(Mutex::new(sensitive_arg));
        let sensitive_arc_copy = sensitive_arc.clone();
        let unwrapped = *sensitive_arc_copy.lock().unwrap();
        println!("{}", unwrapped);
    }
}

mod tricky_flows {
    #[pear::scrutinizer_impure]
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

mod non_leaky_flows {
    #[pear::scrutinizer_pure]
    pub fn foo(sensitive_arg: i32) {
        let mut variable = 1;
        // Implicit flow.
        if variable > 0 {
            variable = 2;
        }
        println!("{}", variable);
    }
}
