mod self_recursive {
    fn pure(a: usize) {
        if a > 0 {
            pure(a - 1);
        }
    }

    fn impure(a: usize) {
        if a > 0 {
            impure(a - 1);
        }
        println!("{}", a);
    }
}

mod mutually_recursive {
    fn pure_1(a: usize) {
        if a > 0 {
            pure_2(a - 1);
        }
    }

    fn pure_2(a: usize) {
        if a > 0 {
            pure_1(a - 1);
        }
    }

    fn impure_1(a: usize) {
        if a > 0 {
            impure_2(a - 1);
        }
        println!("{}", a);
    }

    fn impure_2(a: usize) {
        if a > 0 {
            impure_1(a - 1);
        }
        println!("{}", a);
    }
}
