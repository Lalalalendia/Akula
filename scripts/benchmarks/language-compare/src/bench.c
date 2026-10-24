#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define LIMIT 10000000
#define HASH_N 20000000

int main(void) {
    uint8_t *marked = (uint8_t *)calloc(LIMIT + 1, 1);
    if (!marked) return 2;
    for (int p = 2; p * p <= LIMIT; p++) {
        if (!marked[p]) {
            for (int j = p * p; j <= LIMIT; j += p) marked[j] = 1;
        }
    }
    int count = 0;
    for (int i = 2; i <= LIMIT; i++) count += !marked[i];
    free(marked);
    uint32_t hash = UINT32_C(2166136261);
    for (uint32_t i = 0; i < HASH_N; i++) {
        hash = (hash ^ ((i * 131u + 17u) & 255u)) * UINT32_C(16777619);
    }
    printf("%d\n%u\n", count, hash);
    return 0;
}
