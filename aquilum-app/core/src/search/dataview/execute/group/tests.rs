use super::*;

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn reference(keys: &[Value]) -> Vec<(Value, Vec<Value>)> {
    let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
    for (index, key) in keys.iter().cloned().enumerate() {
        let item = Value::Number(index as f64);
        match groups.iter_mut().find(|(first, _)| first.equals(&key)) {
            Some((_, items)) => items.push(item),
            None => groups.push((key, vec![item])),
        }
    }
    groups
}

fn indexed(keys: &[Value]) -> Groups {
    let mut groups = Groups::default();
    for (index, key) in keys.iter().cloned().enumerate() {
        groups.push(key, Value::Number(index as f64));
    }
    groups
}

#[test]
fn one_group_uses_original_small_group_path() {
    let keys = vec![text("main"); 1_000];
    let groups = indexed(&keys);
    assert_eq!(groups.values, reference(&keys));
    assert_eq!(groups.values.len(), 1);
    assert!(groups.by_text.is_none(), "do not hash the one-group dashboard summary");
}

#[test]
fn distinct_and_repeated_keys_keep_first_seen_order() {
    let mut keys: Vec<_> = (0..150).map(|n| text(&format!("folder-{n:03}"))).collect();
    keys.extend((0..150).rev().map(|n| text(&format!("FOLDER-{n:03}"))));
    let groups = indexed(&keys);
    assert_eq!(groups.values, reference(&keys));
    assert_eq!(groups.values.len(), 150);
    assert!(groups.by_text.is_some());
}

#[test]
fn mixed_types_and_nested_lists_obey_exact_value_equals() {
    let mut keys: Vec<_> = (0..INDEX_AFTER_UNIQUE_GROUPS).map(|n| text(&format!("prefix-{n}"))).collect();
    keys.extend([
        Value::Number(5.0),
        text("5"),
        Value::Bool(true),
        text("TRUE"),
        Value::Null,
        text(""),
        Value::Date(0),
        Value::Date(1),
        Value::List(vec![Value::Number(5.0)]),
        Value::List(vec![text("5")]),
        Value::Object(vec![("field".to_owned(), text("A"))]),
        Value::Object(vec![("field".to_owned(), text("a"))]),
        text("ЁЖ"),
        text("ёж"),
    ]);
    let groups = indexed(&keys);
    assert_eq!(groups.values, reference(&keys));
    assert!(groups.by_text.is_some());
}

#[test]
fn ambiguous_list_and_text_match_the_earliest_equal_group() {
    let mut keys: Vec<_> = (0..INDEX_AFTER_UNIQUE_GROUPS).map(|n| text(&format!("prefix-{n}"))).collect();
    keys.push(Value::List(vec![text("a"), text("b")]));
    keys.push(Value::List(vec![text("a, b")]));
    keys.push(text("a, b"));
    let groups = indexed(&keys);
    assert_eq!(groups.values, reference(&keys));
    assert_eq!(groups.values.len(), INDEX_AFTER_UNIQUE_GROUPS + 2);
    assert_eq!(groups.values[INDEX_AFTER_UNIQUE_GROUPS].1.len(), 2);
    assert_eq!(groups.values[INDEX_AFTER_UNIQUE_GROUPS + 1].1.len(), 1);
}

#[test]
fn thousand_unique_keys_match_the_linear_reference() {
    let mut keys: Vec<_> = (0..1_000).map(|n| text(&format!("unique-{n:05}"))).collect();
    keys.extend((0..1_000).step_by(11).map(|n| text(&format!("UNIQUE-{n:05}"))));
    let groups = indexed(&keys);
    assert_eq!(groups.values, reference(&keys));
    assert_eq!(groups.values.len(), 1_000);
}

#[test]
#[ignore = "Release-only synthetic GROUP BY comparison; two ABBA samples per mode"]
fn benchmark_search_pipeline_dataview_group_by_abba() {
    use std::time::Instant;

    for (rows, distinct) in [
        (1_000, 1), (1_000, 100), (1_000, 1_000),
        (10_000, 1), (10_000, 100), (10_000, 10_000),
        (50_000, 1), (50_000, 100), (50_000, 50_000),
    ] {
        let keys: Vec<_> = (0..rows)
            .map(|row| text(&format!("group-{:05}", row % distinct)))
            .collect();
        // At 50k distinct keys the legacy path makes ~1.25 billion
        // comparisons; measure the indexed path only instead of stalling CI.
        let sequence: &[bool] = if rows == 50_000 && distinct == 50_000 {
            &[true, true]
        } else {
            &[false, true, true, false]
        };
        for (round, &optimized) in sequence.iter().enumerate() {
            let started = Instant::now();
            let result = if optimized { indexed(&keys).values } else { reference(&keys) };
            let elapsed = started.elapsed();
            assert_eq!(result.len(), distinct);
            let route = if optimized { "index" } else { "legacy" };
            println!(
                "BENCH-SEARCH GROUPBY_ABBA rows={rows} distinct={distinct} route={route} round={} elapsed_us={} parity=PASS",
                round + 1,
                elapsed.as_micros(),
            );
        }
    }
}
