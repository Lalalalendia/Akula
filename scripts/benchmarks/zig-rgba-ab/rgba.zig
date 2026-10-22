// Zig 0.17.0: exact counterpart of rgba.rs; no stdlib, no heap allocation.
// Caller supplies nonoverlapping src/dst arrays sized for width/height <= 65535.
fn readWord(src: [*]const u8, i: usize) u32 {
    return @as(u32, src[i])
        | (@as(u32, src[i + 1]) << 8)
        | (@as(u32, src[i + 2]) << 16)
        | (@as(u32, src[i + 3]) << 24);
}
fn writeWord(dst: [*]u8, i: usize, value: u32) void {
    dst[i] = @intCast(value & 255);
    dst[i + 1] = @intCast((value >> 8) & 255);
    dst[i + 2] = @intCast((value >> 16) & 255);
    dst[i + 3] = @intCast((value >> 24) & 255);
}
export fn rgba_half_scalar(src: [*]const u8, dst: [*]u8, width: usize, height: usize) usize {
    if (width == 0 or height == 0 or (width == 1 and height == 1)) return 0;
    const dw = (width + 1) / 2;
    const dh = (height + 1) / 2;
    const max_x = width - 1;
    const max_y = height - 1;
    var transparent = false;
    var y: usize = 0;
    while (y < dh) : (y += 1) {
        const y0 = y * 2;
        const y1 = @min(y0 + 1, max_y);
        var x: usize = 0;
        while (x < dw) : (x += 1) {
            const x0 = x * 2;
            const x1 = @min(x0 + 1, max_x);
            const a = (y0 * width + x0) * 4;
            const b = (y0 * width + x1) * 4;
            const c = (y1 * width + x0) * 4;
            const d = (y1 * width + x1) * 4;
            const out = (y * dw + x) * 4;
            var ch: usize = 0;
            while (ch < 4) : (ch += 1) {
                const sum: u32 = @as(u32, src[a + ch]) + @as(u32, src[b + ch])
                    + @as(u32, src[c + ch]) + @as(u32, src[d + ch]);
                const averaged: u8 = @intCast((sum + 2) / 4);
                dst[out + ch] = averaged;
                if (ch == 3 and averaged != 255) transparent = true;
            }
        }
    }
    return if (transparent) 2 else 1;
}
export fn rgba_half_packed(src: [*]const u8, dst: [*]u8, width: usize, height: usize) usize {
    if (width == 0 or height == 0 or (width == 1 and height == 1)) return 0;
    const dw = (width + 1) / 2;
    const dh = (height + 1) / 2;
    const max_x = width - 1;
    const max_y = height - 1;
    const lanes: u32 = 0x00ff_00ff;
    const round: u32 = 0x0002_0002;
    var transparent = false;
    var y: usize = 0;
    while (y < dh) : (y += 1) {
        const y0 = y * 2;
        const y1 = @min(y0 + 1, max_y);
        var x: usize = 0;
        while (x < dw) : (x += 1) {
            const x0 = x * 2;
            const x1 = @min(x0 + 1, max_x);
            const a = readWord(src, (y0 * width + x0) * 4);
            const b = readWord(src, (y0 * width + x1) * 4);
            const c = readWord(src, (y1 * width + x0) * 4);
            const d = readWord(src, (y1 * width + x1) * 4);
            const even = (a & lanes) + (b & lanes) + (c & lanes) + (d & lanes);
            const odd = ((a >> 8) & lanes) + ((b >> 8) & lanes)
                + ((c >> 8) & lanes) + ((d >> 8) & lanes);
            const reduced = (((even + round) >> 2) & lanes)
                | ((((odd + round) >> 2) & lanes) << 8);
            if (reduced >> 24 != 255) transparent = true;
            writeWord(dst, (y * dw + x) * 4, reduced);
        }
    }
    return if (transparent) 2 else 1;
}
