//! Explicit opt-in Notion API read-only diagnostic.
//! Supply one authorized root UUID as an argument and an ephemeral token through
//! AKULA_NOTION_READ_TOKEN. Never accepts a token or an output path on the CLI.
//! Prints only summary JSON; does not write raw Notion data or Akula vault files.
use aquilum_core::notion_api_probe::{http::HttpReadClient, inspect_root, ProbeLimits};
use std::env;
use uuid::Uuid;

fn main() {
    if let Err(problem) = run() {
        eprintln!("Notion read-only diagnostic refused: {problem}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os();
    let _executable = args.next();
    let root = args.next()
        .ok_or("usage: akula-notion-api-probe <explicit-authorized-root-uuid>")?;
    if args.next().is_some() {
        return Err("expected exactly one root UUID; no token or output path arguments".into());
    }
    let root = root.to_str().ok_or("root UUID must be UTF-8")?;
    let root = Uuid::parse_str(root)
        .map_err(|_| "root must be a valid Notion UUID")?.to_string();

    // This standalone laboratory accepts only a token already made available
    // locally by its operator. Never ask for a token in arguments or in a log.
    let token = env::var("AKULA_NOTION_READ_TOKEN")
        .map_err(|_| "AKULA_NOTION_READ_TOKEN is not configured")?;
    let mut client = HttpReadClient::new(token)
        .map_err(|_| "invalid local Notion credential")?;
    let preview = inspect_root(&mut client, &root, ProbeLimits::default())
        .map_err(|e| format!("Notion read failed without a complete root snapshot: {e}"))?;
    let summary = serde_json::to_string_pretty(&preview.summary())
        .map_err(|_| "could not serialize bounded Notion summary")?;
    println!("{summary}");
    Ok(())
}
