//! Akula's three *exact* getrandom versions, each forced into the binary.
fn main() {
    let mut v2 = [0u8; 32];
    let mut v3 = [0u8; 32];
    let mut v4 = [0u8; 32];
    getrandom02::getrandom(&mut v2).expect("getrandom 0.2 failed");
    getrandom03::fill(&mut v3).expect("getrandom 0.3 failed");
    getrandom04::fill(&mut v4).expect("getrandom 0.4 failed");
    assert!(v2 != v3 || v3 != v4);
    println!("random-ok");
}
