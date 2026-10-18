//! Real subprocess boundary: exit() avoids Rust cleanup and drops all
//! in-memory state. This is *process* failure, NOT power-loss testing.
use staya_filegate_receipt_lab::{filegate_publish, sign_and_persist};
use std::{env, path::Path, process};

fn main() {
    let mut argv = env::args().skip(1);
    let base = argv.next().expect("temp base");
    let scenario = argv.next().expect("scenario");
    let base = Path::new(&base);
    let tag = 1u8;
    match scenario.as_str() {
        "before_gate" => process::exit(80),
        "after_gate" => {
            filegate_publish(base, tag).expect("Core File Gate did not create");
            process::exit(81);
        }
        "after_receipt" => {
            filegate_publish(base, tag).expect("Core File Gate did not create");
            sign_and_persist(base, tag).expect("receipt not durable");
            process::exit(82);
        }
        _ => panic!("unknown crash boundary {scenario}"),
    }
}
