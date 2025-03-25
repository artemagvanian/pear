struct Foo {
    a: usize,
    b: &'static str,
    c: bool,
}

#[pear::scrutinizer_pure]
fn structs(a: usize) {
    let mut foo = Foo {
        a,
        b: "hello",
        c: true,
    };
    foo.a = 30;
    foo.b = "hello2";
}
