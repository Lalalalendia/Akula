//! Diagnostic-only integration benchmark of actual Dataview execution.
//! Copied unchanged into BOTH pinned base and PR worktrees by CI.
use aquilum_core::search::dataview::{execute, parser, rows::Row};
use aquilum_core::search::fields::Field;
use std::{env, time::Instant};

const RUNS: usize = 7;

fn fixture(size: usize) -> Vec<Row> {
    (0..size)
        .map(|id| {
            let mut fields = (0..24)
                .map(|col| {
                    Field::scalar(
                        format!("Field{col:02}"),
                        format!("{}", (id * 11 + col) % 100),
                    )
                })
                .collect::<Vec<_>>();
            fields.push(Field::scalar(
                "Stage".to_owned(),
                if id % 3 == 0 { "active" } else { "inactive" }.to_owned(),
            ));
            fields.push(Field::scalar("Rating".to_owned(), format!("{}", id % 17)));
            fields.push(Field::scalar("Название".to_owned(), format!("Книга {:05}", id % 450)));
            fields.push(Field::scalar("title".to_owned(), format!("Книга {id:05}")));
            let relative = format!("Книги/Книга{id:05}.md");
            Row {
                source: format!("/tmp/akula-dataview-bench/{relative}"),
                relative: relative.clone(),
                name: format!("Книга{id:05}"),
                folder: "Книги".to_owned(),
                created: 1_700_000_000_000_000_000 + id as i64,
                modified: 1_700_000_000_000_000_000 + id as i64,
                size: 256,
                fields,
                tasks: Vec::new(),
            }
        })
        .collect()
}

fn one_case(size: usize, name: &str, query: &str) {
    let parsed = parser::parse(query).expect("valid stable Dataview query");
    // Construction and Rust/SQLite startup are excluded. Cloning one
    // preloaded query row batch is outside the timed execution path.
    let samples = (0..RUNS).map(|_| fixture(size)).collect::<Vec<_>>();
    let mut durations = Vec::with_capacity(RUNS);
    let mut expected_digest = None;
    for rows in samples {
        let start = Instant::now();
        let output = execute::run(&parsed, rows).expect("query completed");
        durations.push(start.elapsed().as_secs_f64() * 1_000.0);
        let serialized = serde_json::to_vec(&output).expect("serializable output");
        let digest = blake3::hash(&serialized);
        if let Some(previous) = expected_digest {
            assert_eq!(digest, previous, "unstable query result");
        } else {
            expected_digest = Some(digest);
        }
    }
    durations.sort_by(f64::total_cmp);
    let p50 = durations[RUNS / 2];
    // With seven observations this is the observed maximum, not a
    // statistically stable latency percentile of the application.
    let p95_upper = durations[RUNS - 1];
    println!(
        "DATAVIEW_QUERY_AB size={} case={} p50_ms={:.6} p95_upper_ms={:.6} digest={}",
        size, name, p50, p95_upper, expected_digest.unwrap().to_hex()
    );
}

#[test]
#[ignore = "diagnostic-only paired release ABBA; not a production performance assertion"]
fn benchmark_real_dataview_query_execution() {
    let size = env::var("AKULA_DATAVIEW_QUERY_SIZE")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(1000);
    assert!([1000, 5000].contains(&size), "unsupported deterministic fixture size");
    one_case(
        size,
        "table_numeric",
        "TABLE WITHOUT ID file.name, FIELD23 WHERE FIELD23 >= 10 SORT FIELD23 DESC LIMIT 50",
    );
    one_case(
        size,
        "list_filter",
        "LIST WITHOUT ID file.name WHERE STAGE = \"active\" SORT RATING DESC LIMIT 50",
    );
    one_case(
        size,
        "table_unicode",
        "TABLE WITHOUT ID file.name, Название, file.path WHERE Название != \"\" SORT Название LIMIT 50",
    );
}
