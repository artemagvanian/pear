fn main() {
    env_logger::init();
    rustc_plugin::cli_main(peirce_frontend::PeircePlugin);
}
