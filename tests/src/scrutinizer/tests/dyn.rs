mod object_type_eraser {
    trait DynamicTrait {
        fn inc(&self, a: usize) -> usize;
    }

    struct PureIncrementer;

    struct ImpureIncrementer;

    impl DynamicTrait for PureIncrementer {
        fn inc(&self, a: usize) -> usize {
            a + 1
        }
    }

    impl DynamicTrait for ImpureIncrementer {
        fn inc(&self, a: usize) -> usize {
            println!("{}", a);
            a + 1
        }
    }

    #[pear::scrutinizer_impure]
    fn dyn_eraser(a: usize) -> usize {
        let dynamic: &dyn DynamicTrait = if a == 0 {
            &PureIncrementer {}
        } else {
            &ImpureIncrementer {}
        };
        dyn_eraser_helper(a, dynamic)
    }

    #[pear::scrutinizer_impure]
    fn dyn_eraser_helper(a: usize, dynamic: &dyn DynamicTrait) -> usize {
        dynamic.inc(a)
    }
}

mod returns_impl_fn {
    #[pear::scrutinizer_pure]
    fn outer(a: usize) -> usize {
        let cl = hof(a);
        execute(a, &cl)
    }

    #[pear::scrutinizer_impure]
    fn execute(a: usize, cl: &dyn Fn(usize) -> usize) -> usize {
        cl(a)
    }

    #[pear::scrutinizer_pure]
    fn hof(a: usize) -> impl Fn(usize) -> usize {
        move |x| x + a
    }
}

mod passthrough_impl_fn {
    #[pear::scrutinizer_pure]
    fn outer(a: usize) -> usize {
        let cl = hof(a);
        execute(a, identity(&cl))
    }

    #[pear::scrutinizer_impure]
    fn execute(a: usize, cl: &dyn Fn(usize) -> usize) -> usize {
        cl(a)
    }

    #[pear::scrutinizer_pure]
    fn hof(a: usize) -> impl Fn(usize) -> usize {
        move |x| x + a
    }

    #[pear::scrutinizer_impure]
    fn identity<T>(a: T) -> T {
        a
    }
}

mod returns_boxed_fn {
    #[pear::scrutinizer_pure]
    fn outer(a: usize) -> usize {
        let cl = hof(a);
        execute(a, &cl)
    }

    #[pear::scrutinizer_impure]
    fn execute(a: usize, cl: &dyn Fn(usize) -> usize) -> usize {
        cl(a)
    }

    #[pear::scrutinizer_pure]
    fn hof(a: usize) -> Box<dyn Fn(usize) -> usize> {
        Box::new(move |x| x + a)
    }
}

mod returns_impl_fn_with_upvars {
    #[pear::scrutinizer_pure]
    fn outer(a: usize) -> usize {
        let lam = |x| x + 1;
        let cl = hof(a, &lam);
        execute(a, &cl)
    }

    #[pear::scrutinizer_impure]
    fn execute(a: usize, cl: &dyn Fn(usize) -> usize) -> usize {
        cl(a)
    }

    #[pear::scrutinizer_impure]
    fn hof(a: usize, cl: &dyn Fn(usize) -> usize) -> impl Fn(usize) -> usize + '_ {
        move |x| cl(x + a)
    }
}

mod returns_boxed_fn_with_upvars {
    #[pear::scrutinizer_pure]
    fn outer(a: usize) -> usize {
        let lam = |x| x + 1;
        let cl = hof(a, &lam);
        execute(a, &cl)
    }

    #[pear::scrutinizer_impure]
    fn execute(a: usize, cl: &dyn Fn(usize) -> usize) -> usize {
        cl(a)
    }

    #[pear::scrutinizer_impure]
    fn hof(a: usize, cl: &dyn Fn(usize) -> usize) -> Box<dyn Fn(usize) -> usize + '_> {
        Box::new(move |x| cl(x + a))
    }
}
