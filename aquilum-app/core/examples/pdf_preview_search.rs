//! A test witness using the same Tantivy search path as the running Akula Core.
//! The preview remains manual and does not enable native PDF index parsing.
use aquilum_core::search::error::SearchError;
use aquilum_core::search::index::SearchIndex;
use aquilum_core::search::progress::IndexProgress;
use aquilum_core::search::query::Request;
use aquilum_core::search::sync::IndexSynchronizer;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn adapt<T>(result: Result<T, SearchError>) -> io::Result<T> {
    result.map_err(|error| io::Error::other(error.to_string()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("missing vault")?).canonicalize()?;
    let storage = PathBuf::from(args.next().ok_or("missing index output")?);
    let query = args.next().ok_or("missing expected word")?.into_string()
        .map_err(|_| "expected word must be UTF-8")?;
    if args.next().is_some() { return Err("unexpected argument".into()); }
    std::fs::create_dir_all(&storage)?;
    let index = Arc::new(adapt(SearchIndex::open(&storage.join("tantivy")))?);
    let progress = Arc::new(IndexProgress::new(0, 1));
    let mut worker = adapt(IndexSynchronizer::new(
        root.clone(), Arc::clone(&index), &storage.join("metadata.sqlite3"), progress,
    ))?;
    adapt(worker.full_scan(&AtomicBool::new(false)))?;
    let found = adapt(index.search(&Request::new(&query)))?;
    let hits = found.results.iter().filter(|hit| {
        let path = PathBuf::from(&hit.path);
        path.starts_with(root.join("PDF Search Preview"))
            && path.extension().and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    }).count();
    if hits != 1 {
        return Err(format!("expected 1 PDF preview hit for {query}, observed {hits}").into());
    }
    println!("AKULA_PDF_SEARCH_E2E=PASS;hits={hits};query={query}");
    Ok(())
}
