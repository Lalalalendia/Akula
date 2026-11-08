mod a11y;
mod args;
mod autostart;
mod app;
mod cover;
mod poster;
mod graph; mod reader;
mod video;
mod dates; mod docx;
#[cfg(feature = "html-preview")] mod html_preview;
#[cfg(feature = "html-preview-shiva")] mod html_preview_shiva;
#[cfg(feature = "dicom-preview")] mod dicom;
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
mod yandex_credentials;
#[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
mod s3_pair_session;
#[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
mod s3_pair_password;
// Inert coordinator in default builds; no network, auth or Sync on startup.
#[allow(dead_code)]
mod yandex_probe_session;
#[cfg(all(target_os = "windows", feature = "yandex-s3-connection-preview"))]
mod yandex_probe_worker;
mod workspace;
