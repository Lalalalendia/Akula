module main;
import std::io;

fn void main() {
    const int LIMIT = 10000000;
    char[] marked = mem::new_array(char, LIMIT + 1);
    defer mem::free(marked.ptr);
    for (int p = 2; p*p <= LIMIT; p++) {
        if (marked[p] == 0) {
            for (int j = p*p; j <= LIMIT; j += p) marked[j] = 1;
        }
    }
    int count = 0;
    for (int i = 2; i <= LIMIT; i++) if (marked[i] == 0) count++;
    uint hash = 2166136261;
    for (uint i = 0; i < 20000000; i++) {
        hash = (hash ^ ((i * 131 + 17) & 255)) * 16777619;
    }
    io::printn(count);
    io::printn(hash);
}
