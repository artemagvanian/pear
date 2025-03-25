use pear_frontend::logging::setup_logger;

fn main() {
    setup_logger().expect("failed to initialize fern");
    rustc_plugin::cli_main(pear_frontend::scrutinizer_plugin::ScrutinizerPlugin);
}
