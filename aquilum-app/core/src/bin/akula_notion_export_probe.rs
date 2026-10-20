//! Explicit offline diagnostic. Stdout: JSON. Stderr: errors only.
use aquilum_core::notion_export::{inspect_path, ScanLimits};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os();
    let _binary = args.next();
    let Some(root) = args.next() else {
        eprintln!("usage: akula-notion-export-probe <notion-zip-or-extracted-directory>");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("expected one input directory; refuses workspace/output path arguments");
        std::process::exit(2);
    }
    let root = PathBuf::from(root);
    match inspect_path(&root, ScanLimits::default()) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("cannot serialize report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("notion inventory refused: {error}");
            std::process::exit(1);
        }
    }
}
