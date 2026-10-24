package main

import "fmt"

func main() {
    const limit = 10000000
    marked := make([]bool, limit+1)
    for p := 2; p*p <= limit; p++ {
        if !marked[p] {
            for j := p*p; j <= limit; j += p { marked[j] = true }
        }
    }
    count := 0
    for i := 2; i <= limit; i++ { if !marked[i] { count++ } }
    var hash uint32 = 2166136261
    for i := uint32(0); i < 20000000; i++ {
        hash = (hash ^ ((i*131 + 17) & 255)) * 16777619
    }
    fmt.Printf("%d\n%d\n", count, hash)
}
