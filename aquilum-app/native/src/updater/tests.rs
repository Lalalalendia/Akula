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
fn independent_akula_signed_update_channel_is_windows_installed_only() {
    assert_eq!(ENDPOINT, "https://github.com/Lalalalendia/Akula/releases/latest/download/latest.json");
    assert!(!PUBLIC_KEY.is_empty(), "owner-pinned Akula public key must be compiled in");
    assert_eq!(
        channel_ready(),
        cfg!(all(target_os = "windows", feature = "installed")),
    );
    assert_eq!(
        available(),
        cfg!(all(target_os = "windows", feature = "installed")),
        "only own signed Windows beta has a live update channel",
    );
}

#[cfg(not(all(target_os = "windows", feature = "installed")))]
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

#[cfg(target_os = "windows")]
#[test]
fn installed_windows_apply_cannot_accept_unsigned_bytes() {
    // Production Windows apply now requires a private-field SignedCandidate.
    // An unverified byte slice is not an executable interface or authority.
    let result = super::release::admit_signed_payload(
        b"{\"version\":\"999.999.999\"}",
        "not a signed manifest",
        b"MZ synthetic unsigned NSIS bytes".to_vec(),
    );
    assert!(result.is_err(), "unsigned bytes must never produce an installer");
    assert_eq!(channel_ready(), cfg!(feature = "installed"));
}

#[test]
fn concurrent_update_claims_allow_exactly_one_worker() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = Arc::clone(&wakes);
    let updater = Updater::new(Arc::new(move || {
        wake_count.fetch_add(1, Ordering::SeqCst);
    }));
    let barrier = Arc::new(Barrier::new(17));
    let mut workers = Vec::new();
    for _ in 0..16 {
        let updater = updater.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            updater.try_start()
        }));
    }
    barrier.wait();
    let claims = workers.into_iter().map(|worker| worker.join().unwrap()).filter(|claimed| *claimed).count();
    assert_eq!(claims, 1, "concurrent calls must not create multiple update workers");
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(updater.status(), Status::Checking);
    assert!(!updater.try_start(), "busy updater may not be claimed again");
}
