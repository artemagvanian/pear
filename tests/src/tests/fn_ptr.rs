pub mod fn_ptr_coercion {
    pub fn foo(data: usize) -> usize {
        data + 1
    }

    pub fn fn_to_fn_ptr(data: usize) -> usize {
        let fn_ptr: fn(usize) -> usize = foo;
        fn_ptr(data)
    }
}

pub mod fmt {
  pub fn format(data: usize) -> String {
      format!("{}", data)
  }
}
