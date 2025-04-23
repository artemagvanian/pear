mod implicit_drop {
    trait T {}

    struct Foo {
        a: u32,
    }

    impl T for Foo {}

    impl Drop for Foo {
        fn drop(&mut self) {
            println!("{}", self.a);
        }
    }

    #[pear::analysis_entry]
    fn implicit_drop_box() {
        let dyn_foo: Box<dyn T> = Box::new(Foo { a: 42 });
    }
}