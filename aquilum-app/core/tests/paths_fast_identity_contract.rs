use aquilum_core::search::paths::{identity, same_path, strip_root};
use std::path::{Path, PathBuf};

fn legacy_same_path(left: &Path, right: &Path) -> bool {
    identity(left) == identity(right)
}

fn legacy_strip_root<'a>(root: &Path, path: &'a Path) -> Option<&'a Path> {
    let mut rest = path.components();
    let inside = root.components().all(|part| {
        rest.next().is_some_and(|other| legacy_same_path(Path::new(&part), Path::new(&other)))
    });
    inside.then_some(rest.as_path())
}

#[test]
fn exact_fast_path_keeps_identity_contract_even_when_paths_compare_equal_as_components() {
    for (left, right) in [
        ("vault/note.md", "vault/note.md"),
        ("vault//note.md", "vault/note.md"),
        ("vault/./note.md", "vault/note.md"),
        ("vault/Мой план.md", "vault/Мои\u{306} план.md"),
        ("vault/Файл.md", "vault/файл.md"),
        (r"\\?\C:\vault\note.md", r"C:\vault\note.md"),
        (r"\\?\UNC\server\share\note.md", r"\\server\share\note.md"),
        ("a/b", "a/b/"),
        ("a/b", "a/c"),
    ] {
        assert_eq!(
            same_path(Path::new(left), Path::new(right)),
            legacy_same_path(Path::new(left), Path::new(right)),
            "comparison changed: {left:?} vs {right:?}",
        );
    }
    assert!(!same_path(Path::new("vault//note.md"), Path::new("vault/note.md")));
}

#[test]
fn fallback_preserves_root_stripping_and_remainder() {
    for (root, path) in [
        ("vault", "vault/notes/book.md"),
        ("vault//notes", "vault/notes/book.md"),
        ("vault/./notes", "vault/notes/book.md"),
        ("vault/Мой", "vault/Мои\u{306}/Идея.md"),
        ("C:/БАЗА", r"c:\база\Проекты\Идея.md"),
        (r"\\?\C:\vault", r"C:\vault\book.md"),
        ("vault/notes", "vault/another/book.md"),
        ("vault/notes/book.md", "vault/notes"),
        ("", "vault/notes"),
        ("vault/notes", "vault/notes"),
    ] {
        assert_eq!(
            strip_root(Path::new(root), Path::new(path)),
            legacy_strip_root(Path::new(root), Path::new(path)),
            "strip changed: {root:?} vs {path:?}",
        );
    }
}

#[test]
fn generated_components_and_nonascii_paths_match_legacy() {
    let fragments = ["vault", "vault//", "vault/.", "Проекты", "мой", "мои\u{306}", ".заметки", "nested"];
    let paths: Vec<PathBuf> = fragments.iter().flat_map(|root| {
        fragments.iter().map(move |leaf| Path::new(root).join(leaf).join("Идея.md"))
    }).collect();
    for left in &paths {
        for right in &paths {
            assert_eq!(same_path(left, right), legacy_same_path(left, right));
            assert_eq!(strip_root(left, right), legacy_strip_root(left, right));
        }
    }
}

#[test]
#[ignore = "optional same-runner Release ABBA path equality microbenchmark"]
fn benchmark_search_pipeline_paths_abba() {
    use std::hint::black_box;
    use std::time::Instant;

    let equal = Path::new("vault/Projects/long-folder/Идея.md");
    let normalized = Path::new("vault/Projects/long-folder/Мои\u{306}.md");
    let composed = Path::new("vault/Projects/long-folder/Мой.md");
    let root = Path::new("vault/Projects/long-folder");
    let path = Path::new("vault/Projects/long-folder/Идея.md");
    let cases = [
        ("same_path", "exact", equal, equal),
        ("same_path", "unicode", normalized, composed),
        ("same_path", "different", equal, composed),
        ("strip_root", "exact", root, path),
        ("strip_root", "miss", Path::new("another"), path),
    ];
    const ITERATIONS: usize = 20_000;
    for (kind, scenario, left, right) in cases {
        let expected = if kind == "same_path" {
            legacy_same_path(left, right)
        } else {
            legacy_strip_root(left, right).is_some()
        };
        for (round, fast) in [false, true, true, false].into_iter().enumerate() {
            let start = Instant::now();
            let mut found = 0usize;
            for _ in 0..ITERATIONS {
                let yes = if kind == "same_path" {
                    if fast { same_path(black_box(left), black_box(right)) }
                    else { legacy_same_path(black_box(left), black_box(right)) }
                } else if fast {
                    strip_root(black_box(left), black_box(right)).is_some()
                } else {
                    legacy_strip_root(black_box(left), black_box(right)).is_some()
                };
                found += usize::from(black_box(yes));
            }
            let elapsed = start.elapsed();
            assert_eq!(found, ITERATIONS * usize::from(expected));
            println!(
                "BENCH-SEARCH PATH_ABBA kind={kind} scenario={scenario} route={} round={} iterations={ITERATIONS} elapsed_us={} parity=PASS",
                if fast { "fast" } else { "legacy" },
                round + 1,
                elapsed.as_micros(),
            );
        }
    }
}
