use staya_fed_inventory_lab::{
    compare, full_compare, sample_revision, Inventory, Revision, TEST_VAULT,
};

fn report(name: &str, left: &[Revision], right: &[Revision]) {
    let a = Inventory::fixture(TEST_VAULT, true, left.to_vec()).expect("valid a");
    let b = Inventory::fixture(TEST_VAULT, true, right.to_vec()).expect("valid b");
    let result = compare(&a, &b, 100_000).expect("budgeted bounded comparison");
    let baseline = full_compare(&a, &b).expect("independent comparison");
    assert_eq!(result.missing_from_a, baseline.missing_from_a);
    assert_eq!(result.missing_from_b, baseline.missing_from_b);
    assert_eq!(result.collisions, baseline.collisions);
    println!(
        "FED_INVENTORY_01 case={name} nodes={} leaves={} estimated_bytes={} full_list_bytes={} missing_from_a={} missing_from_b={} changed={} read_only=true real_wire=false signed_authorization=false",
        result.stats.nodes_compared,
        result.stats.leaf_exchanges,
        result.stats.estimated_bytes,
        result.stats.full_list_bytes,
        result.missing_from_a.len(),
        result.missing_from_b.len(),
        result.collisions.len()
    );
}

fn main() {
    let all: Vec<_> = (0..10_000).map(sample_revision).collect();
    report("same", &all, &all);
    let one_missing: Vec<_> = (0..10_000).filter(|i| *i != 1337).map(sample_revision).collect();
    report("one_missing", &all, &one_missing);
    let one_percent_missing: Vec<_> = (100..10_000).map(sample_revision).collect();
    report("one_percent_missing", &all, &one_percent_missing);
    let mut b = one_percent_missing.clone();
    b.extend((10_000..10_050).map(sample_revision));
    report("hundred_missing_plus_fifty_new", &all, &b);
    let half: Vec<_> = (0..10_000).filter(|i| i % 2 == 0).map(sample_revision).collect();
    report("half_missing", &all, &half);
    let disjoint: Vec<_> = (10_000..20_000).map(sample_revision).collect();
    report("fully_disjoint", &all, &disjoint);
}
