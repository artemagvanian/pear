use pear_frontend::logging::setup_logger;

fn main() {
    setup_logger().expect("failed to initialize fern");
    rustc_plugin::driver_main(pear_frontend::pear_plugin::PearPlugin);
}
