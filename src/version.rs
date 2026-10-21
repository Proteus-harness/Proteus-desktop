pub const LABEL: &str = concat!(
    "Proteus native ",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("PROTEUS_BUILD_COMMIT"),
    ")"
);

pub fn requested() -> bool {
    let mut args = std::env::args_os().skip(1);
    matches!(args.next().as_deref(), Some(arg) if arg == "--version" || arg == "-V")
        && args.next().is_none()
}
