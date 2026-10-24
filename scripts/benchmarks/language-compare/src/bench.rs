fn main() {
    const LIMIT: usize = 10_000_000;
    let mut marked = vec![false; LIMIT + 1];
    let mut p = 2;
    while p * p <= LIMIT {
        if !marked[p] {
            let mut j = p * p;
            while j <= LIMIT {
                marked[j] = true;
                j += p;
            }
        }
        p += 1;
    }
    let count = (2..=LIMIT).filter(|&i| !marked[i]).count();
    let mut hash: u32 = 2166136261;
    for i in 0u32..20_000_000u32 {
        hash = (hash ^ (i.wrapping_mul(131).wrapping_add(17) & 255)).wrapping_mul(16777619);
    }
    println!("{}\n{}", count, hash);
}
