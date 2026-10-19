// This includes the *extracted real* impl Row from the exact base/head rows.rs.
// Only the surrounding Field/Row data shapes are substituted to remove SQL and
// unrelated crate startup from an isolated lookup microbenchmark.
#[derive(Clone)]
struct Field {
    key: String,
    ordinal: u32,
}
struct Row {
    fields: Vec<Field>,
}
include!(env!("AKULA_DATAVIEW_IMPL"));

use std::hint::black_box;
use std::time::Instant;

fn input(with_unicode: bool) -> Row {
    let mut fields = (0..32).map(|i| Field {
        key: format!("Field{i:02}"),
        ordinal: i,
    }).collect::<Vec<_>>();
    if with_unicode {
        fields.insert(4, Field { key: "Key".to_owned(), ordinal: 40 });
        fields.insert(5, Field { key: "ΟΣ".to_owned(), ordinal: 41 });
        fields.insert(6, Field { key: "İ".to_owned(), ordinal: 42 });
    }
    Row { fields }
}

fn bench(name: &str) {
    let unicode = name == "mixed_unicode";
    let row = input(unicode);
    let queries: &[&str] = match name {
        "ascii_late" => &["FIELD31", "field30", "Field28", "FIELD27"],
        "ascii_miss" => &["absent", "missing", "notfound", "unknown"],
        "mixed_unicode" => &["KEY", "KEY", "ος", "ΟΣ", "i\u{307}", "missing", "FIELD31"],
        _ => unreachable!(),
    };
    let mut checksum = 0xCBF2_9CE4_8422_2325_u64;
    let started = Instant::now();
    for i in 0..40_000 {
        let needle = black_box(queries[i % queries.len()]);
        let id = row.field(needle).map(|f| f.ordinal).unwrap_or(u32::MAX);
        checksum = (checksum ^ id as u64).wrapping_mul(0x1000_0000_01B3);
    }
    let ms = started.elapsed().as_secs_f64() * 1_000.0;
    black_box(checksum);
    println!("DATAVIEW_FIELD_AB case={name} elapsed_ms={ms:.6} checksum={checksum}");
}

fn main() {
    for name in ["ascii_late", "ascii_miss", "mixed_unicode"] {
        bench(name);
    }
}
