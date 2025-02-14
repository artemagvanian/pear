pub fn setup_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {}] {}",
                record.level(),
                record.target(),
                message,
            ))
        })
        .level(log::LevelFilter::Warn)
        .level_for("peirce_backend", log::LevelFilter::Debug)
        .level_for("peirce_frontend", log::LevelFilter::Debug)
        .chain(fern::log_file("output.peirce.log")?)
        .apply()?;
    Ok(())
}
