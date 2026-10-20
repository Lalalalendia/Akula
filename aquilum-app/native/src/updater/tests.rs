use super::release::{channel_ready, ENDPOINT, PUBLIC_KEY, newer};
use super::{available, Status, Updater};
use std::sync::Arc;

#[test]
fn versions_compare_numerically() {
    assert!(newer("0.2.10", "0.2.9"));
    assert!(newer("v0.3.0", "0.2.6"));
    assert!(!newer("0.2.6", "0.2.6"));
    assert!(!newer("0.2.5", "0.2.6"));
}

#[test]
fn no_upstream_update_channel_is_available() {
    assert_eq!(ENDPOINT, "https://github.com/Lalalalendia/Akula/releases/latest/download/latest.json");
    assert!(PUBLIC_KEY.is_empty(), "an Akula signing key must be provisioned separately");
    assert!(!channel_ready());
    assert!(!available(), "installed builds must not offer an upstream Aquilum update");
}

#[test]
fn disabled_update_channel_never_starts_network_work() {
    let updater = Updater::new(Arc::new(|| {}));
    updater.check();
    assert_eq!(updater.status(), Status::Idle);
    updater.install();
    assert_eq!(updater.status(), Status::Idle);
}

#[cfg(target_os = "linux")]
#[test]
fn installed_linux_apply_refuses_unauthorized_bytes_without_shutdown() {
    let mut shutdown_called = false;
    let result = super::apply(b"untrusted fake AppImage", || {
        shutdown_called = true;
    });
    let error = result.unwrap_err();
    assert!(error.contains("HOLD"), "Linux apply must refuse absent publisher and GUI ACK");
    assert!(!shutdown_called, "Linux apply cannot stop Akula before trusted admission");
}
