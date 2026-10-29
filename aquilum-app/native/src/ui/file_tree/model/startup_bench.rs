use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{Sort, TreeModel};

fn fixture(root: &Path, notes: usize, nested: bool) -> Vec<PathBuf> {
    if !nested {
        for i in 0..notes {
            std::fs::File::create(root.join(format!("note_{i:06}.md"))).unwrap();
        }
        return Vec::new();
    }
    let mut expanded = Vec::new();
    for group in 0..10 {
        let folder = root.join(format!("folder_{group:02}"));
        std::fs::create_dir_all(&folder).unwrap();
        expanded.push(folder.clone());
        for i in 0..notes / 10 {
            std::fs::File::create(folder.join(format!("note_{group:02}_{i:06}.md"))).unwrap();
        }
    }
    expanded
}

fn original(root: &Path, expanded: &[PathBuf], sort: Sort) -> TreeModel {
    let mut model = TreeModel::new(Some(root.to_path_buf()), expanded.to_vec());
    model.set_sort(sort);
    model
}

fn candidate(root: &Path, expanded: &[PathBuf], sort: Sort) -> TreeModel {
    TreeModel::with_saved_sort(Some(root.to_path_buf()), expanded.to_vec(), sort)
}

#[test]
fn saved_sort_initialization_matches_postload_sort_for_all_modes() {
    let dir = crate::test_dir::tempdir();
    let root = dir.path();
    let folder = root.join("Вложенная");
    std::fs::create_dir(&folder).unwrap();
    for file in ["Бета.md", "Альфа.md", "image.png"] {
        std::fs::File::create(root.join(file)).unwrap();
    }
    std::fs::File::create(folder.join("Глубоко.md")).unwrap();
    for sort in Sort::ALL {
        let expanded = [folder.clone()];
        let a = original(root, &expanded, sort);
        let b = candidate(root, &expanded, sort);
        assert_eq!(a.rows(), b.rows(), "saved sort {sort:?} changed rows");
        assert_eq!(a.times, b.times, "saved sort {sort:?} changed file timestamps");
        assert_eq!(a.sort(), b.sort());
    }
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn percentile(times: &[Duration], pct: usize) -> f64 {
    let mut values = times.to_vec();
    values.sort();
    let index = (values.len() * pct).div_ceil(100).saturating_sub(1);
    micros(values[index])
}

// Paired release-profile microbenchmark: measures synchronous TreeModel startup,
// not the first HWND, background Counts::scan or installed GUI J2 time.
// 40 alternating paired samples make the p95 more meaningful than the N=8 pilot.
#[test]
#[ignore = "opt-in FileTree startup saved-sort A/B, no time gate"]
fn benchmark_filetree_saved_sort_abba() {
    for (notes, nested) in [(1_000, false), (10_000, false), (10_000, true)] {
        let dir = crate::test_dir::tempdir();
        let root = dir.path();
        let expanded = fixture(root, notes, nested);
        let layout = if nested { "expanded" } else { "flat" };
        for sort in [Sort::NameDesc] {
            let (mut before, mut after) = (Vec::new(), Vec::new());
            let mut rows = 0usize;
            for round in 0..40 {
                let sample = |new: bool| {
                    let t = Instant::now();
                    let model = if new {
                        candidate(root, &expanded, sort)
                    } else {
                        original(root, &expanded, sort)
                    };
                    (model, t.elapsed())
                };
                let (old, new) = if round % 2 == 0 {
                    (sample(false), sample(true))
                } else {
                    let new = sample(true);
                    let old = sample(false);
                    (old, new)
                };
                assert_eq!(old.0.rows(), new.0.rows(), "notes={notes} layout={layout} sort={sort:?}");
                assert_eq!(old.0.times, new.0.times, "timestamp metadata changed");
                rows = new.0.rows().len();
                before.push(old.1);
                after.push(new.1);
            }
            println!(
                "FILETREE_STARTUP_SORT_AB notes={notes} layout={layout} sort={sort:?} rows={rows} old_p50_us={:.2} new_p50_us={:.2} old_p95_us={:.2} new_p95_us={:.2} rounds=40 parity=PASS",
                percentile(&before, 50), percentile(&after, 50),
                percentile(&before, 95), percentile(&after, 95),
            );
        }
    }
}
