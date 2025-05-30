mod fn_trait {
    fn fn_1(a: usize, b: usize) -> usize {
        a + b
    }

    fn fn_2(a: usize, b: usize) -> usize {
        a - b
    }

    fn invoker(func: &dyn Fn(usize, usize) -> usize, a: usize, b: usize) -> usize {
        func(a, b)
    }

    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;

        let func = if a > b {
            &fn_1 as &dyn Fn(usize, usize) -> usize
        } else {
            &fn_2 as &dyn Fn(usize, usize) -> usize
        };

        let res = invoker(func, a, b);
    }
}

mod fn_mut_trait {
    fn fn_1(a: usize, b: usize) -> usize {
        a + b
    }

    fn fn_2(a: usize, b: usize) -> usize {
        a - b
    }

    fn invoker(func: &mut dyn FnMut(usize, usize) -> usize, a: usize, b: usize) -> usize {
        func(a, b)
    }

    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;

        let func_1 = &mut fn_1 as &mut dyn FnMut(usize, usize) -> usize;
        let func_2 = &mut fn_2 as &mut dyn FnMut(usize, usize) -> usize;

        let func = if a > b { func_1 } else { func_2 };

        let res = invoker(func, a, b);
    }
}

mod fn_trait_closure {
    fn invoker(func: &dyn Fn(usize, usize) -> usize, a: usize, b: usize) -> usize {
        func(a, b)
    }

    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;

        let fn_1 = |a: usize, b: usize| -> usize { a + b };
        let fn_2 = |a: usize, b: usize| -> usize { a - b };

        let func = if a > b {
            &fn_1 as &dyn Fn(usize, usize) -> usize
        } else {
            &fn_2 as &dyn Fn(usize, usize) -> usize
        };

        let res = invoker(func, a, b);
    }
}

mod boxed_fn_trait {
    fn fn_1(a: usize, b: usize) -> usize {
        a + b
    }

    fn fn_2(a: usize, b: usize) -> usize {
        a - b
    }

    fn invoker(func: Box<dyn Fn(usize, usize) -> usize>, a: usize, b: usize) -> usize {
        func(a, b)
    }

    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;

        let func_1: Box<dyn Fn(usize, usize) -> usize> = Box::new(fn_1);
        let func_2: Box<dyn Fn(usize, usize) -> usize> = Box::new(fn_2);

        let func = if a > b { func_1 } else { func_2 };

        let res = invoker(func, a, b);
    }
}

mod boxed_closure_as_fn_trait {
    fn fn_1(a: usize, b: usize) -> usize {
        a + b
    }

    fn fn_2(a: usize, b: usize) -> usize {
        a - b
    }

    fn invoker(func: Box<dyn Fn(usize, usize) -> usize>, a: usize, b: usize) -> usize {
        func(a, b)
    }

    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;

        let func_1: Box<dyn Fn(usize, usize) -> usize> = Box::new(|a: usize, b: usize| fn_1(a, b));
        let func_2: Box<dyn Fn(usize, usize) -> usize> = Box::new(|a: usize, b: usize| fn_2(a, b));

        let func = if a > b { func_1 } else { func_2 };

        let res = invoker(func, a, b);
    }
}
