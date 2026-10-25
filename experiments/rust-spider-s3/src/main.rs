//! A deliberately narrow proof-of-concept: Spider -> S3 PutObject with content hashes.
//! This is NOT a general cloud storage client nor a product-ready sync engine.
use anyhow::{bail, Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::{primitives::ByteStream, Client};
use sha2::{Digest, Sha256};
use spider::website::Website;
use std::{env, sync::{atomic::{AtomicUsize, Ordering}, Arc}};
use tokio::{sync::Semaphore, task::JoinSet};

fn required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing environment variable {name}"))
}

async fn store(
    client: Client,
    bucket: String,
    prefix: String,
    name: String,
    content: Vec<u8>,
    uploads: Arc<AtomicUsize>,
    crash_after: usize,
) -> Result<()> {
    let key = format!("{prefix}/{name}");
    let digest = hex::encode(Sha256::digest(&content));
    let listed = client.list_objects_v2()
        .bucket(&bucket).prefix(&key).max_keys(2).send().await
        .with_context(|| format!("list S3 object {key}"))?;
    if listed.contents().iter().any(|obj| obj.key() == Some(key.as_str())) {
        let head = client.head_object().bucket(&bucket).key(&key).send().await
            .with_context(|| format!("head S3 object {key}"))?;
        let matches = head.metadata().and_then(|m| m.get("sha256"))
            .is_some_and(|existing| existing == &digest);
        if !matches { bail!("existing object has different SHA-256 metadata: {key}"); }
        println!("SKIP {key} {digest}");
        return Ok(());
    }
    client.put_object()
        .bucket(&bucket)
        .key(&key)
        .metadata("sha256", &digest)
        .content_type("text/html; charset=utf-8")
        .body(ByteStream::from(content))
        .send().await.with_context(|| format!("put S3 object {key}"))?;
    let n = uploads.fetch_add(1, Ordering::SeqCst) + 1;
    println!("PUT {key} {digest} {n}");
    if crash_after != 0 && n >= crash_after {
        // Intentionally crash *after* S3 accepted the object, before any local journal commit.
        std::process::exit(87);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = required("TRIAL_ROOT")?;
    let endpoint = required("S3_ENDPOINT")?;
    let bucket = required("S3_BUCKET")?;
    let prefix = required("S3_PREFIX")?;
    let crash_after = env::var("CRASH_AFTER_PUT_N").unwrap_or_default().parse::<usize>().unwrap_or(0);
    let n_pages = env::var("TRIAL_PAGES").unwrap_or_default().parse::<u32>().unwrap_or(160);
    let shared = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .load().await;
    let config = aws_sdk_s3::config::Builder::from(&shared)
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();
    let client = Client::from_conf(config);
    let uploads = Arc::new(AtomicUsize::new(0));
    let mut website = Website::new(&root);
    website.with_depth(2);
    website.with_limit(n_pages + 1);
    website.with_concurrency_limit(Some(4));
    website.with_respect_robots_txt(false);
    website.with_user_agent(Some("rust-s3-compare/0.1"));
    let mut receiver = website.subscribe(8);
    let transfer = tokio::spawn(async move {
        let sem = Arc::new(Semaphore::new(4));
        let mut tasks = JoinSet::new();
        while let Ok(page) = receiver.recv().await {
            if !page.get_url().contains("/pages/") { continue; }
            if !page.status_code.is_success() {
                bail!("page failed HTTP status {}: {}", page.status_code, page.get_url());
            }
            let name = page.get_url().rsplit('/').next().context("missing filename")?.to_owned();
            let body = page.get_html_bytes_u8().to_vec();
            if body.is_empty() { bail!("empty HTML bytes for {name}"); }
            let permit = sem.clone().acquire_owned().await?;
            let client = client.clone();
            let bucket = bucket.clone();
            let prefix = prefix.clone();
            let uploads = uploads.clone();
            tasks.spawn(async move {
                let _permit = permit;
                store(client, bucket, prefix, name, body, uploads, crash_after).await
            });
            // Surface completed task errors before waiting for every in-flight upload.
            while let Some(done) = tasks.try_join_next() { done??; }
        }
        while let Some(done) = tasks.join_next().await { done??; }
        Ok::<(), anyhow::Error>(())
    });
    website.crawl().await;
    website.unsubscribe();
    transfer.await??;
    println!("FINISHED rust Spider crawl");
    Ok(())
}
