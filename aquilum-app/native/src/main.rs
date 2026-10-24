#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Do not accidentally package an experimental live network control. QA may
// compile it only in non-release builds; production release requires separate
// acceptance, code change and review.
#[cfg(all(feature = "yandex-s3-connection-preview", not(debug_assertions)))]
compile_error!("Yandex S3 connection preview must not ship in release builds");

macro_rules! report {
    ($($arg:tt)*) => {
        $crate::host::report(&format!($($arg)*))
    };
}

include!("native_modules.rs");

pub use args::Args;
fn main() { if let Some(exit_code) = pdf_worker::maybe_run() { std::process::exit(exit_code); }
    args::run_w4_pilot_if_requested();    #[cfg(target_os = "windows")]
    unsafe {
        #[link(name = "winmm")]
        unsafe extern "system" {
            fn timeBeginPeriod(uPeriod: u32) -> u32;
        }
        let _ = timeBeginPeriod(1);
    }
    if std::env::args().any(|argument| argument == "--mcp-stdio") {
        std::process::exit(aquilum_core::mcp::run_stdio_bridge(workspace::APP_ID));
    }
    let result = args::parse().and_then(host::run);
    if let Err(err) = &result {
        report!("{err}");
        rfd::MessageDialog::new().set_title(if cfg!(any(target_os = "windows", target_os = "linux")) { "Akula" } else { "Aquilum" }).set_description(err).set_level(rfd::MessageLevel::Error).show();
    }
    std::process::exit(i32::from(result.is_err()));
}
