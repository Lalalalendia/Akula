#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

macro_rules! report {
    ($($arg:tt)*) => {
        $crate::host::report(&format!($($arg)*))
    };
}

mod a11y;
mod args;
mod autostart;
mod app;
mod cover;
mod poster;
mod graph;
mod reader;
mod video;
mod dates;
mod disk;
mod file_ops;
mod fling;
mod frontmatter;
mod i18n;
mod images;
mod instance;
mod interface;
mod links;
mod history;
mod host;
mod metrics;
#[cfg(feature = "messaging-w4-pilot")]
mod messaging_w4_pilot;
#[cfg(feature = "memory-probe")]
mod memory_probe;

#[cfg(feature = "memory-probe")]
#[global_allocator]
static ALLOCATOR: memory_probe::Counting = memory_probe::Counting;
mod navigation;
mod note;
mod output;
mod paths;
mod pdf;
mod rank;
mod rope;
mod stats;
mod render;
#[cfg(not(feature = "installed"))]
mod replay;
mod session;
mod system;
mod tabs;
#[cfg(target_os = "windows")]
mod touchpad;
#[cfg(not(target_os = "windows"))]
mod touchpad {
    pub struct Touchpad;

    impl Touchpad {
        pub fn attach(_: &dyn winit::window::Window) -> Option<Self> {
            None
        }

        pub fn active(&self) -> bool {
            false
        }

        pub fn update(&self, _: std::time::Instant, _: f64) -> (f32, f32, f32) {
            (0.0, 0.0, 1.0)
        }
    }
}
#[cfg(test)]
mod test_dir;
mod vault_files;
mod vsync;
mod ui;
mod updater;
mod window_state;
mod workspace;

pub use args::Args;

fn main() {
    // Explicit opt-in: normal Akula startup never starts a messaging listener.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--w4-pilot")) {
        #[cfg(feature = "messaging-w4-pilot")]
        std::process::exit(messaging_w4_pilot::run());
        #[cfg(not(feature = "messaging-w4-pilot"))]
        {
            eprintln!("W4 пилот недоступен в этой сборке Акулы");
            std::process::exit(2);
        }
    }
    #[cfg(target_os = "windows")]
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
