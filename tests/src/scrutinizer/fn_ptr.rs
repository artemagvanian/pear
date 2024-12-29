mod fn_ptr_coercion {
    fn foo(data: usize) -> usize {
        data + 1
    }

    fn fn_to_fn_ptr(data: usize) -> usize {
        let fn_ptr: fn(usize) -> usize = foo;
        fn_ptr(data)
    }
}

mod fmt {
  fn format(data: usize) -> String {
      format!("{}", data)
  }
}
