mod dyn_ref {
    trait Foo {
        fn bar(&self, a: usize, b: usize) -> usize;
    }

    struct S1;
    struct S2;

    impl Foo for S1 {
        fn bar(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl Foo for S2 {
        fn bar(&self, a: usize, b: usize) -> usize {
            a - b
        }
    }
    
    fn invoker(s: &dyn Foo, a: usize, b: usize) -> usize {
        s.bar(a, b)
    }
    
    #[peirce::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let s = if a > b {
            &S1 {} as &dyn Foo
        } else {
            &S2 {} as &dyn Foo
        };
        
        let res = invoker(s, a, b);
    }
}

mod box_dyn_ref {
    trait Foo {
        fn bar(&self, a: usize, b: usize) -> usize;
    }

    struct S1;
    struct S2;

    impl Foo for S1 {
        fn bar(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl Foo for S2 {
        fn bar(&self, a: usize, b: usize) -> usize {
            a - b
        }
    }
    
    fn invoker(s: Box<dyn Foo>, a: usize, b: usize) -> usize {
        s.bar(a, b)
    }
    
    #[peirce::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let s = if a > b {
            Box::new(S1 {}) as Box<dyn Foo>
        } else {
            Box::new(S2 {}) as Box<dyn Foo>
        };
        
        let res = invoker(s, a, b);
    }
}

mod dyn_super_trait {
    trait Bar {
        fn super_bar(&self, a: usize, b: usize) -> usize;
    }

    trait Foo: Bar {
        fn bar(&self, a: usize, b: usize) -> usize;
    }

    struct S1;
    struct S2;

    impl Bar for S1 {
        fn super_bar(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl Bar for S2 {
        fn super_bar(&self, a: usize, b: usize) -> usize {
            a - b
        }
    }

    impl Foo for S1 {
        fn bar(&self, a: usize, b: usize) -> usize {
            a + 2 * b
        }
    }

    impl Foo for S2 {
        fn bar(&self, a: usize, b: usize) -> usize {
            a - 2 * b
        }
    }
    
    fn invoker(s: &dyn Foo, a: usize, b: usize) -> usize {
        s.super_bar(a, b)
    }
    
    #[peirce::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let s = if a > b {
            &S1 {} as &dyn Foo
        } else {
            &S2 {} as &dyn Foo
        };
        
        let res = invoker(s, a, b);
    }
}

mod dyn_different_vtables {
    trait A: B {
        fn a(&self, a: usize, b: usize) -> usize;
    }

    trait B {
        fn b(&self, a: usize, b: usize) -> usize;
    }

    struct S1;
    struct S2;

    impl B for S1 {
        fn b(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl B for S2 {
        fn b(&self, a: usize, b: usize) -> usize {
            a - b
        }
    }

    impl A for S1 {
        fn a(&self, a: usize, b: usize) -> usize {
            a + 2 * b
        }
    }
    
    fn invoker(s: &dyn B, a: usize, b: usize) -> usize {
        s.b(a, b)
    }
    
    #[peirce::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let s1 = &S1 {} as &dyn A;
        let s2 = &S2 {} as &dyn B;

        let res = if a > b {
            invoker(s1, a, b)
        } else {
            invoker(s2, a, b)
        };
    }
}

mod dyn_diamond {
    // Trait dependency graph is as follows:
    //    A
    //  /   \
    // B     C
    //  \   /
    //    D

    trait A : B + C {
        fn a(&self, a: usize, b: usize) -> usize;
    }

    trait B : D {
        fn b(&self, a: usize, b: usize) -> usize;
    }

    trait C: D {
        fn c(&self, a: usize, b: usize) -> usize;
    }

    trait D {
        fn d(&self, a: usize, b: usize) -> usize;
    }

    struct S1;
    struct S2;

    impl A for S1 {
        fn a(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl B for S1 {
        fn b(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl C for S1 {
        fn c(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl D for S1 {
        fn d(&self, a: usize, b: usize) -> usize {
            a + b
        }
    }

    impl C for S2 {
        fn c(&self, a: usize, b: usize) -> usize {
            a - b
        }
    }

    impl D for S2 {
        fn d(&self, a: usize, b: usize) -> usize {
            a - b
        }
    }
    
    fn invoker(s: &dyn C, a: usize, b: usize) -> usize {
        s.c(a, b)
    }
    
    #[peirce::analysis_entry]
    fn main() {
        let a = 5;
        let b = 6;
        
        let s1 = &S1 {} as &dyn A;
        let s2 = &S2 {} as &dyn C;

        let res = if a > b {
            invoker(s1, a, b)
        } else {
            invoker(s2, a, b)
        };
    }
}