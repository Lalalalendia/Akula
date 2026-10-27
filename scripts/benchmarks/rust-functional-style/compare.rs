//! Single-run research: Rust imperative loop vs iterator chain.
//! Same task and input. No production code changes or third-party dependencies.
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone, Copy)]
struct Book {
    published: bool,
    ru: bool,
    chapters: u32,
}

#[inline(never)]
fn imperative(books: &[Book]) -> u64 {
    let mut total = 0_u64;
    for book in books {
        if book.published && book.ru {
            total += u64::from(book.chapters);
        }
    }
    total
}

#[inline(never)]
fn functional(books: &[Book]) -> u64 {
    books
        .iter()
        .filter(|book| book.published && book.ru)
        .map(|book| u64::from(book.chapters))
        .sum()
}

fn books_fixture(count: usize) -> Vec<Book> {
    let mut books = Vec::with_capacity(count);
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..count {
        // Reproducible xorshift input, not a random benchmark result.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let x = seed as u32;
        books.push(Book {
            published: x & 3 != 0,
            ru: (x >> 2) & 3 == 0,
            chapters: 1 + ((x >> 8) % 300),
        });
    }
    books
}

fn run_one_pass_many_times(
    books: &[Book],
    f: fn(&[Book]) -> u64,
    repetitions: usize,
) -> f64 {
    let start = Instant::now();
    let mut checksum = 0_u64;
    for _ in 0..repetitions {
        let input = black_box(books);
        checksum = checksum.wrapping_add(black_box(f(input)));
    }
    black_box(checksum);
    (start.elapsed().as_secs_f64() * 1_000.0) / repetitions as f64
}

fn median(sorted: &[f64]) -> f64 {
    let len = sorted.len();
    if len % 2 == 0 {
        (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0
    } else {
        sorted[len / 2]
    }
}

fn p95(sorted: &[f64]) -> f64 {
    let idx = ((sorted.len() * 95).div_ceil(100)).saturating_sub(1);
    sorted[idx]
}

fn main() {
    const BOOKS: usize = 10_000_000;
    const REPETITIONS: usize = 32;
    const TRIALS: usize = 60;

    let books = books_fixture(BOOKS);
    let expected = imperative(&books);
    assert_eq!(functional(&books), expected);

    for _ in 0..3 {
        let _ = run_one_pass_many_times(&books, imperative, REPETITIONS);
        let _ = run_one_pass_many_times(&books, functional, REPETITIONS);
    }

    let mut imperative_times = Vec::with_capacity(TRIALS);
    let mut functional_times = Vec::with_capacity(TRIALS);
    let mut csv = String::from("trial,variant,ms_per_full_scan\n");

    for trial in 0..TRIALS {
        // Alternate AB / BA order to reduce warm-cache and clock bias.
        let (a, b) = if trial % 2 == 0 {
            (
                run_one_pass_many_times(&books, imperative, REPETITIONS),
                run_one_pass_many_times(&books, functional, REPETITIONS),
            )
        } else {
            let b = run_one_pass_many_times(&books, functional, REPETITIONS);
            let a = run_one_pass_many_times(&books, imperative, REPETITIONS);
            (a, b)
        };
        imperative_times.push(a);
        functional_times.push(b);
        csv.push_str(&format!("{trial},imperative,{a:.6}\n"));
        csv.push_str(&format!("{trial},functional,{b:.6}\n"));
    }

    imperative_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    functional_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let a_med = median(&imperative_times);
    let b_med = median(&functional_times);
    let a_p95 = p95(&imperative_times);
    let b_p95 = p95(&functional_times);

    println!("BOOKS={BOOKS} REPETITIONS={REPETITIONS} TRIALS={TRIALS}");
    println!("FULL_SCANS_PER_VARIANT={}", REPETITIONS * TRIALS);
    println!("RECORDS_PROCESSED_PER_VARIANT={}", (BOOKS as u64) * (REPETITIONS as u64) * (TRIALS as u64));
    println!("EXPECTED_CHAPTERS={expected}");
    println!("BOOK_SIZE_BYTES={}", std::mem::size_of::<Book>());
    println!("IMPERATIVE_MEDIAN_MS={a_med:.6} IMPERATIVE_P95_MS={a_p95:.6}");
    println!("FUNCTIONAL_MEDIAN_MS={b_med:.6} FUNCTIONAL_P95_MS={b_p95:.6}");
    println!("FUNCTIONAL_TO_IMPERATIVE_RATIO={:.6}", b_med / a_med);

    std::fs::create_dir_all("rust-functional-results").unwrap();
    let summary = format!(
        "{{\n  \"fixture\": \"deterministic xorshift\",\n  \"books\": {BOOKS},\n  \"repetitions_per_trial\": {REPETITIONS},\n  \"trials\": {TRIALS},\n  \"chapters\": {expected},\n  \"book_size_bytes\": {},\n  \"imperative_median_ms\": {a_med:.6},\n  \"imperative_p95_ms\": {a_p95:.6},\n  \"functional_median_ms\": {b_med:.6},\n  \"functional_p95_ms\": {b_p95:.6},\n  \"functional_to_imperative_ratio\": {:.6}\n}}\n",
        std::mem::size_of::<Book>(),
        b_med / a_med
    );
    std::fs::write("rust-functional-results/summary.json", summary).unwrap();
    std::fs::write("rust-functional-results/trials.csv", csv).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        assert_eq!(imperative(&[]), 0);
        assert_eq!(functional(&[]), 0);
    }

    #[test]
    fn mixed_books_have_known_answer() {
        let books = [
            Book { published: true, ru: true, chapters: 12 },
            Book { published: false, ru: true, chapters: 90 },
            Book { published: true, ru: false, chapters: 80 },
            Book { published: true, ru: true, chapters: 31 },
        ];
        assert_eq!(imperative(&books), 43);
        assert_eq!(functional(&books), 43);
    }

    #[test]
    fn sum_is_u64_not_u32() {
        let books = [
            Book { published: true, ru: true, chapters: u32::MAX },
            Book { published: true, ru: true, chapters: u32::MAX },
        ];
        assert_eq!(imperative(&books), 2 * u64::from(u32::MAX));
        assert_eq!(functional(&books), 2 * u64::from(u32::MAX));
    }

    #[test]
    fn large_fixture_parity() {
        let books = books_fixture(100_000);
        assert_eq!(imperative(&books), functional(&books));
    }
}
