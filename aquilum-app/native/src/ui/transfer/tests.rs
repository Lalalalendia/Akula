use super::*;

use super::*;

#[test]
fn byte_size_always_refers_to_source_metadata_not_network_progress() {
    assert_eq!(display_size(0), "0 B");
    assert_eq!(display_size(1024), "1.0 KiB");
    assert_eq!(display_size(1_048_576), "1.0 MiB");
}
