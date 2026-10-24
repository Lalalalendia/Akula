package main

import "core:fmt"

main :: proc() {
    LIMIT :: 10_000_000
    marked := make([]u8, LIMIT + 1)
    defer delete(marked)
    p := 2
    for p*p <= LIMIT {
        if marked[p] == 0 {
            j := p*p
            for j <= LIMIT {
                marked[j] = 1
                j += p
            }
        }
        p += 1
    }
    count := 0
    for i in 2..=LIMIT {
        if marked[i] == 0 {count += 1}
    }
    hash: u32 = 2166136261
    for i: u32 = 0; i < 20_000_000; i += 1 {
        hash = (hash ~ ((i * 131 + 17) & 255)) * 16777619
    }
    fmt.println(count)
    fmt.println(hash)
}
