mod fn_ptr_coercion {
    #[pear::scrutinizer_pure]
    pub fn foo(data: usize) -> usize {
        data + 1
    }

    #[pear::scrutinizer_pure]
    pub fn fn_to_fn_ptr(data: usize) -> usize {
        let fn_ptr: fn(usize) -> usize = foo;
        fn_ptr(data)
    }
}

mod fmt {
    #[pear::scrutinizer_pure]
    pub fn format(data: usize) -> String {
        format!("{}", data)
    }
}
