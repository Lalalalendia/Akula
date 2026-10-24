//! Explicit offline diagnostic. Stdout: JSON metadata only. Stderr: errors.
use aquilum_core::notion_export::{content::stage_read_only, inspect_path, plan::prepare_plan, ScanLimits};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os();
    let _binary = args.next();
    let Some(first) = args.next() else {
        eprintln!("usage: akula-notion-export-probe [--plan|--verify-content] <zip-or-directory>");
        std::process::exit(2);
    };
    let (mode, input) = match first.to_str() {
        Some("--plan") => ("plan", args.next()),
        Some("--verify-content") => ("verify-content", args.next()),
        _ => ("inventory", Some(first)),
    };
    let Some(root) = input else {
        eprintln!("usage: akula-notion-export-probe [--plan|--verify-content] <zip-or-directory>");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("expected one input ZIP or directory; refuses workspace/output paths");
        std::process::exit(2);
    }
    let root = PathBuf::from(root);
    let limits = ScanLimits::default();
    let result = match mode {
        "verify-content" => stage_read_only(&root, limits).and_then(|staged| {
            serde_json::to_string_pretty(&staged.receipt())
                .map_err(|_| "cannot encode verified stage receipt".into())
        }),
        "plan" | "inventory" => inspect_path(&root, limits).and_then(|report| {
            if mode == "plan" {
                serde_json::to_string_pretty(&prepare_plan(&report))
            } else {
                serde_json::to_string_pretty(&report)
            }.map_err(|_| "cannot encode Notion inventory report".into())
        }),
        _ => Err("unrecognized read-only mode".into()),
    };
    match result {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("Notion read-only diagnostic refused: {error}");
            std::process::exit(1);
        }
    }
}
