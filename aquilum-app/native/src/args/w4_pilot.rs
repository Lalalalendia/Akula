//! Explicit CLI bridge for the signed W4 laboratory in the existing Akula app.
#[cfg(feature = "messaging-w4-pilot")]
#[path = "../messaging_w4_pilot.rs"]
mod messaging_w4_pilot;

/// No side effects for ordinary GUI starts. In production feature-off builds
/// an explicit W4 attempt is refused rather than silently opening a GUI.
pub fn run_w4_pilot_if_requested() {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new("--w4-pilot")) {
        return;
    }
    #[cfg(feature = "messaging-w4-pilot")]
    std::process::exit(messaging_w4_pilot::run());
    #[cfg(not(feature = "messaging-w4-pilot"))]
    {
        eprintln!("W4 недоступен в данной сборке Акулы");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn regular_binary_retains_independent_args_parser() {
        assert!(super::super::parse().is_ok());
    }
}
