#[test]
fn compiled_contracts_do_not_enable_cloud_sync_by_default() {
    assert!(!super::SYNC_ENGINE_FEATURE_ENABLED);
}
