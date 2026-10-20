//! Separate deny-only Windows research binary; never used by Akula native.
//! No command installs a service, provisions host state or issues P2P grants.

fn main() {
    #[cfg(windows)]
    {
        let arg = std::env::args().nth(1);
        let outcome = match arg.as_deref() {
            Some("--service") => akula_host_broker_lab::windows_scm_service::dispatch_scm_service(),
            Some("--service-pipe") => akula_host_broker_lab::windows_scm_service::dispatch_scm_pipe_service(),
            // SCM-only SYSTEM + Service SID diagnostic, no pipe or IPC.
            Some("--verify-scm") => akula_host_broker_lab::windows_client_attestation::
                inspect_installed_scm_process_without_pipe(),
            // Full origin verification intentionally FAILS: no service pipe.
            Some("--verify-pipe") => akula_host_broker_lab::windows_client_attestation::
                inspect_installed_service_pipe_without_sending(),
            None => akula_host_broker_lab::windows_server::run_one_denied_connection(),
            Some(_) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput, "unknown research-only subcommand",
            )),
        };
        if let Err(error) = outcome {
            eprintln!("Akula standalone SCM lab refused: {error}");
            std::process::exit(2);
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("Windows-only research component, not installed by Akula");
        std::process::exit(2);
    }
}
