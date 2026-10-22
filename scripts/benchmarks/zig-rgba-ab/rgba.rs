// Isolated port of aquilum-app/native/src/render/paint/mipmap.rs::half.
// SAFETY: caller supplies valid, nonoverlapping source (w*h*4 bytes) and
// destination (ceil(w/2)*ceil(h/2)*4 bytes), width and height <= 65535.
// Return: 0 = source has no mip; 1 = opaque result; 2 = transparent result.
#[inline(always)]
unsafe fn read_word(src: *const u8, i: usize) -> u32 {
    u32::from_le_bytes([
        *src.add(i), *src.add(i + 1), *src.add(i + 2), *src.add(i + 3),
    ])
}
#[inline(always)]
unsafe fn write_word(dst: *mut u8, i: usize, v: u32) {
    let bytes = v.to_le_bytes();
    for (c, &b) in bytes.iter().enumerate() {
        *dst.add(i + c) = b;
    }
}
#[no_mangle]
pub unsafe extern "C" fn rgba_half_scalar(
    src: *const u8, dst: *mut u8, width: usize, height: usize,
) -> usize {
    if width == 0 || height == 0 || (width == 1 && height == 1) { return 0; }
    let (dw, dh) = (width.div_ceil(2), height.div_ceil(2));
    let (max_x, max_y) = (width - 1, height - 1);
    let mut transparent = false;
    for y in 0..dh {
        let y0 = y * 2;
        let y1 = (y0 + 1).min(max_y);
        for x in 0..dw {
            let x0 = x * 2;
            let x1 = (x0 + 1).min(max_x);
            let a = (y0 * width + x0) * 4;
            let b = (y0 * width + x1) * 4;
            let c = (y1 * width + x0) * 4;
            let d = (y1 * width + x1) * 4;
            let out = (y * dw + x) * 4;
            for channel in 0..4 {
                let sum = u32::from(*src.add(a + channel))
                    + u32::from(*src.add(b + channel))
                    + u32::from(*src.add(c + channel))
                    + u32::from(*src.add(d + channel));
                let averaged = ((sum + 2) / 4) as u8;
                *dst.add(out + channel) = averaged;
                if channel == 3 && averaged != 255 { transparent = true; }
            }
        }
    }
    if transparent { 2 } else { 1 }
}
#[no_mangle]
pub unsafe extern "C" fn rgba_half_packed(
    src: *const u8, dst: *mut u8, width: usize, height: usize,
) -> usize {
    if width == 0 || height == 0 || (width == 1 && height == 1) { return 0; }
    let (dw, dh) = (width.div_ceil(2), height.div_ceil(2));
    let (max_x, max_y) = (width - 1, height - 1);
    let mut transparent = false;
    const LANES: u32 = 0x00ff_00ff;
    const ROUND: u32 = 0x0002_0002;
    for y in 0..dh {
        let y0 = y * 2;
        let y1 = (y0 + 1).min(max_y);
        for x in 0..dw {
            let x0 = x * 2;
            let x1 = (x0 + 1).min(max_x);
            let a = read_word(src, (y0 * width + x0) * 4);
            let b = read_word(src, (y0 * width + x1) * 4);
            let c = read_word(src, (y1 * width + x0) * 4);
            let d = read_word(src, (y1 * width + x1) * 4);
            let even = (a & LANES) + (b & LANES) + (c & LANES) + (d & LANES);
            let odd = ((a >> 8) & LANES) + ((b >> 8) & LANES)
                + ((c >> 8) & LANES) + ((d >> 8) & LANES);
            let reduced = (((even + ROUND) >> 2) & LANES)
                | ((((odd + ROUND) >> 2) & LANES) << 8);
            transparent |= reduced >> 24 != 255;
            write_word(dst, (y * dw + x) * 4, reduced);
        }
    }
    if transparent { 2 } else { 1 }
}
