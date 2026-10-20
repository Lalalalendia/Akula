//! Never installed or invoked by the Akula desktop application.
//! To become a Windows service, a separately audited installer/SCM lifecycle,
//! protected state and IPC client trust must be added in a later stage.

fn main() {
    #[cfg(windows)]
    if let Err(error) = akula_host_broker_lab::windows_server::run_one_denied_connection() {
        eprintln!("non-authorizing Akula broker probe refused: {error}");
        std::process::exit(2);
    }

    #[cfg(not(windows))]
    {
        eprintln!("Akula broker probe is Windows-only and always disabled here");
        std::process::exit(2);
    }
}
