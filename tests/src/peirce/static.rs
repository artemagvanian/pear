mod static_closure {
    static FN: fn(usize, usize) -> usize = |a: usize, b: usize| a + b;

    #[peirce::analysis_entry]
    fn main() {
        let s = FN(5, 5);
    }
}

mod static_fn {
    static FUNC: fn(usize, usize) -> usize = {
        fn fn_1(a: usize, b: usize) -> usize {
            a + b
        }

        fn fn_2(a: usize, b: usize) -> usize {
            a - b
        }

        fn fn_3(a: usize, b: usize) -> usize {
            if a < b {
                fn_1(a, b)
            } else {
                fn_2(a, b)
            }
        }

        fn_3
    };

    #[peirce::analysis_entry]
    fn main() {
        let s = FUNC(5, 5);
    }
}

