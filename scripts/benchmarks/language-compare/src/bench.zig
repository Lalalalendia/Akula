const std = @import("std");

pub fn main() !void {
    const limit: usize = 10_000_000;
    const marked = try std.heap.page_allocator.alloc(u8, limit + 1);
    defer std.heap.page_allocator.free(marked);
    @memset(marked, 0);
    var p: usize = 2;
    while (p * p <= limit) : (p += 1) {
        if (marked[p] == 0) {
            var j: usize = p * p;
            while (j <= limit) : (j += p) marked[j] = 1;
        }
    }
    var count: usize = 0;
    for (marked[2..]) |b| { if (b == 0) count += 1; }
    var hash: u32 = 2166136261;
    var i: u32 = 0;
    while (i < 20_000_000) : (i += 1) {
        hash = (hash ^ ((i *% 131 +% 17) & 255)) *% 16777619;
    }
    std.debug.print("{d}\n{d}\n", .{ count, hash });
}
