mod fn_ptr {
    fn fn_1(a: usize, b: usize) -> usize {
        a + b
    }
    
    fn fn_2(a: usize, b: usize) -> usize {
        a - b
    }
    
    fn invoker(func: fn(usize, usize) -> usize, a: usize, b: usize) -> usize {
        func(a, b)
    }
    
    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let func = if a > b {
            fn_1 as fn(usize, usize) -> usize
        } else {
            fn_2 as fn(usize, usize) -> usize
        };
        
        let res = invoker(func, a, b);
    }
}

mod stored_fn_ptr {
    struct FnPtrWrapper(fn(usize, usize) -> usize);

    impl FnPtrWrapper {
        fn eval(&self, a: usize, b: usize) -> usize {
            self.0(a, b)
        }
    }

    fn fn_1(a: usize, b: usize) -> usize {
        a + b
    }
    
    fn fn_2(a: usize, b: usize) -> usize {
        a - b
    }
    
    #[pear::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let func = if a > b {
            FnPtrWrapper(fn_1 as fn(usize, usize) -> usize)
        } else {
            FnPtrWrapper(fn_2 as fn(usize, usize) -> usize)
        };
        
        let res = func.eval(a, b);
    }
}
