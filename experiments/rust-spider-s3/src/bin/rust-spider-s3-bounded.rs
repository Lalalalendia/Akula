//! Disk-spooled URL input -> bounded fetch -> bounded S3 writer.
//! The original Spider broadcast-based benchmark remains a separate binary.
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    rust_spider_s3_trial::bounded::run_from_env().await
}
