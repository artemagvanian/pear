mod self_recursive {
    #[pear::scrutinizer_pure]
    fn pure(a: usize) {
        if a > 0 {
            pure(a - 1);
        }
    }

    #[pear::scrutinizer_impure]
    fn impure(a: usize) {
        if a > 0 {
            impure(a - 1);
        }
        println!("{}", a);
    }
}

mod mutually_recursive {
    #[pear::scrutinizer_pure]
    fn pure_1(a: usize) {
        if a > 0 {
            pure_2(a - 1);
        }
    }

    #[pear::scrutinizer_pure]
    fn pure_2(a: usize) {
        if a > 0 {
            pure_1(a - 1);
        }
    }

    #[pear::scrutinizer_impure]
    fn impure_1(a: usize) {
        if a > 0 {
            impure_2(a - 1);
        }
        println!("{}", a);
    }

    #[pear::scrutinizer_impure]
    fn impure_2(a: usize) {
        if a > 0 {
            impure_1(a - 1);
        }
        println!("{}", a);
    }
}
