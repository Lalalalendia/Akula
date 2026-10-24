#![forbid(unsafe_code)]
// This crate compiles and tests Akula's ACTUAL adapter files but does not
// depend on aquilum-core, so unrelated CRDT/yrs versions cannot mask results.
#[path = "../../src/public_cdn_lab.rs"]
pub mod public_cdn_lab;
#[cfg(feature = "s3")]
#[path = "../../src/public_cdn_s3_lab.rs"]
pub mod public_cdn_s3_lab;
#[cfg(feature = "bao")]
#[path = "../../src/public_cdn_bao_lab.rs"]
pub mod public_cdn_bao_lab;
