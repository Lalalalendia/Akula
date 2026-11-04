use aquilum_core::search::note_date;

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

struct ResetOffset;
impl Drop for ResetOffset {
    fn drop(&mut self) {
        note_date::set_local_offset(0);
    }
}

fn previous_start_of_day(nanos: i64) -> i64 {
    let (year, month, day, _) = note_date::to_civil(nanos);
    note_date::from_civil(year, month, day, 0)
}

#[test]
fn day_floor_matches_civil_roundtrip_at_epoch_boundaries_and_offsets() {
    let _reset = ResetOffset;
    let values = [
        i64::MIN + 3 * NANOS_PER_DAY,
        -3_000_000_000_000_000_001,
        -NANOS_PER_DAY - 1,
        -NANOS_PER_DAY,
        -NANOS_PER_DAY + 1,
        -1_000_000_001,
        -1_000_000_000,
        -1,
        0,
        1,
        999_999_999,
        1_000_000_000,
        NANOS_PER_DAY - 1,
        NANOS_PER_DAY,
        NANOS_PER_DAY + 1,
        3_000_000_000_000_000_001,
        i64::MAX - 3 * NANOS_PER_DAY,
    ];
    for offset in [-50_400, -43_200, -18_000, -1, 0, 1, 19_800, 50_400] {
        note_date::set_local_offset(offset);
        for nanos in values {
            let actual = note_date::start_of_day(nanos);
            let expected = previous_start_of_day(nanos);
            assert_eq!(actual, expected, "nanos={nanos}, UTC offset={offset}");
            assert_eq!(note_date::start_of_day(actual), actual, "not idempotent");
            assert!(actual <= nanos, "day floor after input timestamp");
        }
    }
}

#[test]
fn day_floor_matches_previous_calendar_algorithm_across_four_centuries() {
    let _reset = ResetOffset;
    // nanosecond i64 only spans ~1677–2262. Exercise thousands of
    // near-midnight, negative, leap-year and fractional-second timestamps.
    for offset in [-43_200, -25_200, -1, 0, 1, 5 * 3600 + 1800, 14 * 3600] {
        note_date::set_local_offset(offset);
        for step in -2000i64..=2000 {
            let nanos = step * (NANOS_PER_DAY / 7) + (step.abs() % 997);
            assert_eq!(
                note_date::start_of_day(nanos),
                previous_start_of_day(nanos),
                "step={step}, offset={offset}",
            );
        }
        for iso in [
            "1690-02-28", "1800-03-01", "1900-03-01", "1969-12-31",
            "1970-01-01", "2000-02-29", "2024-02-29", "2026-10-28", "2250-01-01",
        ] {
            let midnight = note_date::parse(iso).expect("valid in-range ISO day");
            assert_eq!(note_date::start_of_day(midnight), midnight, "{iso}, offset={offset}");
            let civil = note_date::to_civil(midnight);
            assert_eq!(civil.3, 0, "midnight time component");
        }
    }
}

#[test]
#[ignore = "diagnostic Release comparison of exact same date semantics"]
fn benchmark_search_pipeline_start_of_day_abba() {
    use std::hint::black_box;
    use std::time::Instant;

    const ITERATIONS: i64 = 40_000;
    let scenarios = [
        ("epoch-boundary", NANOS_PER_DAY - 1, 0),
        ("before-epoch", -12 * NANOS_PER_DAY + 123, -7 * 3600),
        ("positive-offset", 1_700_000_000_123_000_000, 19_800),
        ("negative-offset", -2_300_000_001_230_000_000, -43_200),
    ];
    let _reset = ResetOffset;
    for (case, value, offset) in scenarios {
        note_date::set_local_offset(offset);
        let expected = previous_start_of_day(value);
        assert_eq!(note_date::start_of_day(value), expected);
        for cycle in 0..8 {
            for (phase, candidate) in [false, true, true, false].into_iter().enumerate() {
                let began = Instant::now();
                let mut checksum = 0i64;
                for _ in 0..ITERATIONS {
                    let result = if candidate {
                        note_date::start_of_day(black_box(value))
                    } else {
                        previous_start_of_day(black_box(value))
                    };
                    checksum = checksum.wrapping_add(black_box(result));
                }
                let elapsed_us = began.elapsed().as_micros();
                assert_eq!(checksum, expected.wrapping_mul(ITERATIONS));
                println!(
                    "DAY_FLOOR_ABBA case={case} route={} cycle={} phase={} iterations={ITERATIONS} elapsed_us={elapsed_us} parity=PASS",
                    if candidate { "fast" } else { "legacy" }, cycle + 1, phase + 1
                );
            }
        }
    }
}
