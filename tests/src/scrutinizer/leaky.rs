mod print {
    fn println_side_effect(left: usize, right: usize) -> usize {
        println!("{} {}", left, right);
        left + right
    }
}

mod network {
    use std::io;
    use std::net::UdpSocket;

    fn udp_socket_send(socket: &UdpSocket, buf: &[u8]) -> io::Result<usize> {
        socket.send(buf)
    }
}

mod interior {
    use std::cell::RefCell;

    fn ref_cell_mut(refcell: &RefCell<usize>) {
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

    fn sneaky_drop(data: usize) {
        let sp = CustomSmartPointer { data };
    }
}

mod adversarial {
    use std::ptr;

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
    
    fn transmute_struct(value: u32, sink: StructImmut) {
        let sink_mut: StructMut = unsafe { std::mem::transmute(sink) };
        *sink_mut.field = value;
    }

    fn transmute_arr(value: u32, sink: [&u32; 1]) {
        let sink_mut: [&mut u32; 1] = unsafe { std::mem::transmute(sink) };
        *sink_mut[0] = value;
    }
}