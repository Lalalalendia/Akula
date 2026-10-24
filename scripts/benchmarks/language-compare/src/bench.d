import std.stdio;

void main() {
    enum limit = 10_000_000;
    bool[] marked = new bool[limit + 1];
    for (int p = 2; p * p <= limit; ++p) {
        if (!marked[p]) {
            for (int j = p * p; j <= limit; j += p) marked[j] = true;
        }
    }
    int count = 0;
    for (int i = 2; i <= limit; ++i) if (!marked[i]) ++count;
    uint hash = 2166136261u;
    for (uint i = 0; i < 20_000_000u; ++i)
        hash = (hash ^ ((i * 131u + 17u) & 255u)) * 16777619u;
    writeln(count);
    writeln(hash);
}
