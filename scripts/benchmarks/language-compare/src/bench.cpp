#include <cstdint>
#include <iostream>
#include <vector>

constexpr int LIMIT = 10000000;
constexpr std::uint32_t HASH_N = 20000000;

int main() {
    std::vector<std::uint8_t> marked(LIMIT + 1, 0);
    for (int p = 2; p * p <= LIMIT; ++p) {
        if (!marked[p]) for (int j = p * p; j <= LIMIT; j += p) marked[j] = 1;
    }
    int count = 0;
    for (int i = 2; i <= LIMIT; ++i) count += !marked[i];
    std::uint32_t hash = 2166136261u;
    for (std::uint32_t i = 0; i < HASH_N; ++i)
        hash = (hash ^ ((i * 131u + 17u) & 255u)) * 16777619u;
    std::cout << count << '\n' << hash << '\n';
}
