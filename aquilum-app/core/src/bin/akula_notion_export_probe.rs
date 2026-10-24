//! Explicit offline diagnostic. Stdout: JSON. Stderr: errors only.
use aquilum_core::notion_export::{inspect_path, plan::prepare_plan, ScanLimits};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os();
    let _binary = args.next();
    let Some(first) = args.next() else {
        eprintln!("usage: akula-notion-export-probe [--plan] <notion-zip-or-extracted-directory>");
        std::process::exit(2);
    };
    let (plan, input) = if first.to_str() == Some("--plan") {
        (true, args.next())
    } else {
        (false, Some(first))
    };
    let Some(root) = input else {
        eprintln!("usage: akula-notion-export-probe [--plan] <notion-zip-or-extracted-directory>");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("expected one input ZIP or directory; refuses workspace/output paths");
        std::process::exit(2);
    }
    let root = PathBuf::from(root);
    match inspect_path(&root, ScanLimits::default()) {
        Ok(report) => {
            let json = if plan {
                serde_json::to_string_pretty(&prepare_plan(&report))
            } else {
                serde_json::to_string_pretty(&report)
            };
            match json {
                Ok(json) => println!("{json}"),
                Err(_) => {
                    eprintln!("cannot serialize read-only Notion diagnostic");
                    std::process::exit(1);
                }
            }
        }
        Err(error) => {
            eprintln!("notion inventory refused: {error}");
            std::process::exit(1);
        }
    }
}
