// Experimental kernels, not part of the installed application.
include!("rgba.rs");

#[inline(always)]
unsafe fn load4(src: *const u8, pos: usize) -> u32 {
    u32::from_le(std::ptr::read_unaligned(src.add(pos).cast::<u32>()))
}
#[inline(always)]
unsafe fn store4(dst: *mut u8, pos: usize, value: u32) {
    std::ptr::write_unaligned(dst.add(pos).cast::<u32>(), value.to_le());
}
#[inline(always)]
fn avg4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    const MASK: u32 = 0x00ff_00ff;
    const ROUND: u32 = 0x0002_0002;
    let evens = (a & MASK) + (b & MASK) + (c & MASK) + (d & MASK);
    let odds = ((a >> 8) & MASK) + ((b >> 8) & MASK)
        + ((c >> 8) & MASK) + ((d >> 8) & MASK);
    (((evens + ROUND) >> 2) & MASK)
        | ((((odds + ROUND) >> 2) & MASK) << 8)
}

/// Same math and indices as original packed, but unaligned u32 I/O.
/// Caller guarantees nonoverlapping valid buffers and width,height <= 65535.
#[no_mangle]
pub unsafe extern "C" fn rgba_half_unaligned(
    src: *const u8, dst: *mut u8, width: usize, height: usize
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
            let a = load4(src, (y0 * width + x0) * 4);
            let b = load4(src, (y0 * width + x1) * 4);
            let c = load4(src, (y1 * width + x0) * 4);
            let d = load4(src, (y1 * width + x1) * 4);
            let result = avg4(a, b, c, d);
            transparent |= result >> 24 != 255;
            store4(dst, (y * dw + x) * 4, result);
        }
    }
    if transparent { 2 } else { 1 }
}

/// Same unaligned I/O but row stride and output offset hoisted.
#[no_mangle]
pub unsafe extern "C" fn rgba_half_row(
    src: *const u8, dst: *mut u8, width: usize, height: usize
) -> usize {
    if width == 0 || height == 0 || (width == 1 && height == 1) { return 0; }
    let (dw, dh) = (width.div_ceil(2), height.div_ceil(2));
    let (max_x, max_y) = (width - 1, height - 1);
    let (src_stride, dst_stride) = (width * 4, dw * 4);
    let mut transparent = false;
    for y in 0..dh {
        let r0 = (2 * y) * src_stride;
        let r1 = (2 * y + 1).min(max_y) * src_stride;
        let dr = y * dst_stride;
        for x in 0..dw {
            let col0 = (2 * x) * 4;
            let col1 = (2 * x + 1).min(max_x) * 4;
            let result = avg4(
                load4(src, r0 + col0), load4(src, r0 + col1),
                load4(src, r1 + col0), load4(src, r1 + col1));
            transparent |= result >> 24 != 255;
            store4(dst, dr + x * 4, result);
        }
    }
    if transparent { 2 } else { 1 }
}
