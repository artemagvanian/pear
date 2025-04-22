mod print {
    #[pear::scrutinizer_impure]
    pub fn println_side_effect(left: usize, right: usize) -> usize {
        println!("{} {}", left, right);
        left + right
    }
}

mod network {
    use std::io;
    use std::net::UdpSocket;

    #[pear::scrutinizer_impure]
    pub fn udp_socket_send(socket: &UdpSocket, buf: &[u8]) -> io::Result<usize> {
        socket.send(buf)
    }
}

mod interior {
    use std::cell::RefCell;

    #[pear::scrutinizer_impure]
    pub fn ref_cell_mut(refcell: &RefCell<usize>) {
        *refcell.borrow_mut() = 10;
    }
}

mod implicit {
    struct CustomSmartPointer {
        data: usize,
    }

    impl Drop for CustomSmartPointer {
        fn drop(&mut self) {
            println!("Dropping CustomSmartPointer with data `{}`!", self.data);
        }
    }

    #[pear::scrutinizer_impure]
    pub fn sneaky_drop(data: usize) {
        let sp = CustomSmartPointer { data };
    }
}

mod adversarial {
    use std::ptr;

    #[pear::scrutinizer_impure]
    unsafe fn intrinsic_leaker(value: &u64, sink: &u64) {
        let sink = sink as *const u64;
        ptr::copy(value as *const u64, sink as *mut u64, 1);
    }

    struct StructImmut<'a> {
        field: &'a u32,
    }
    
    struct StructMut<'a> {
        field: &'a mut u32,
    }
    
    #[pear::scrutinizer_impure]
    fn transmute_struct(value: u32, sink: StructImmut) {
        let sink_mut: StructMut = unsafe { std::mem::transmute(sink) };
        *sink_mut.field = value;
    }

    #[pear::scrutinizer_impure]
    fn transmute_arr(value: u32, sink: [&u32; 1]) {
        let sink_mut: [&mut u32; 1] = unsafe { std::mem::transmute(sink) };
        *sink_mut[0] = value;
    }
}

mod leaky_no_args {
    #[pear::scrutinizer_impure]
    fn leak_conditional(s: String) {
        if s.len() > 0 {
            print_something();
        }
    }

    #[pear::scrutinizer_impure]
    fn leak_conditional_some_args(s: String) {
        if s.len() > 0 {
            print_something_one_arg(42);
        }
    }

    #[pear::scrutinizer_pure]
    fn print_something() {
        println!("foo");
    }

    #[pear::scrutinizer_pure]
    fn print_something_one_arg(a: u32) {
        println!("foo");
    }
}
