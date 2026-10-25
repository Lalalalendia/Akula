//! Strictly read-only PDF raster subprocess, never used on the window thread.
//! IMPORTANT: process separation is NOT an OS sandbox for untrusted PDF files.
use std::ffi::{c_char, c_int, c_void};
use std::path::Path;

const MAX_INPUT: u64 = 256 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;

pub fn maybe_run() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).and_then(|arg| arg.to_str()) != Some("--pdf-worker") {
        return None;
    }
    let result = process(&args);
    if let Err(error) = &result {
        if let Some(status) = args.get(7) {
            let _ = std::fs::write(status, format!("ERROR {}\n", error.replace(['\r', '\n'], " ")));
        }
    }
    Some(i32::from(result.is_err()))
}

fn process(args: &[std::ffi::OsString]) -> Result<(), String> {
    if args.len() != 8 {
        return Err("Invalid PDF worker argument count".into());
    }
    let source = Path::new(&args[2]);
    let page: usize = args[3].to_string_lossy().parse().map_err(|_| "Invalid page")?;
    let target_width: u32 = args[4].to_string_lossy().parse().map_err(|_| "Invalid width")?;
    let rotation: c_int = args[5].to_string_lossy().parse().map_err(|_| "Invalid rotation")?;
    if !(320..=3200).contains(&target_width) || !(0..=3).contains(&rotation) {
        return Err("PDF raster dimensions or rotation exceed limits".into());
    }
    let output = Path::new(&args[6]);
    let status = Path::new(&args[7]);
    let metadata = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || !(16..=MAX_INPUT).contains(&metadata.len()) {
        return Err("PDF must be a regular file no larger than 256 MiB".into());
    }
    let bytes = std::fs::read(source).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != metadata.len() || !bytes.starts_with(b"%PDF-") {
        return Err("Invalid or changed PDF source".into());
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (bytes, page, output, status);
        Err("Native PDF rendering is unsupported on this platform".into())
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        let pdfium = Pdfium::open()?;
        let doc = unsafe { (pdfium.load_document)(bytes.as_ptr().cast(), bytes.len() as c_int, std::ptr::null()) };
        if doc.is_null() {
            return Err("PDF could not be opened (password, corruption or unsupported format)".into());
        }
        let result = render_document(&pdfium, doc, page, target_width, rotation, output, status);
        unsafe { (pdfium.close_document)(doc) };
        result
    }
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn render_document(
    pdfium: &Pdfium, document: *mut c_void, requested: usize, target_width: u32,
    rotation: c_int, output: &Path, status: &Path,
) -> Result<(), String> {
    let count = unsafe { (pdfium.page_count)(document) };
    if !(1..=100_000).contains(&count) || requested >= count as usize {
        return Err("PDF page does not exist or count exceeds limit".into());
    }
    let page = unsafe { (pdfium.load_page)(document, requested as c_int) };
    if page.is_null() {
        return Err("PDF page could not be loaded".into());
    }
    let rendered = rasterize(pdfium, page, target_width, rotation, output);
    unsafe { (pdfium.close_page)(page) };
    let (width, height) = rendered?;
    std::fs::write(status, format!("PDF_VIEW_V1 {count} {width} {height}\n")).map_err(|e| e.to_string())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn rasterize(pdfium: &Pdfium, page: *mut c_void, target_width: u32, rotation: c_int, output: &Path) -> Result<(u32, u32), String> {
    let (mut natural_width, mut natural_height) =
        (unsafe { (pdfium.page_width)(page) }, unsafe { (pdfium.page_height)(page) });
    if !natural_width.is_finite() || !natural_height.is_finite()
        || natural_width <= 0.0 || natural_height <= 0.0 {
        return Err("PDF page has invalid dimensions".into());
    }
    if rotation % 2 != 0 {
        std::mem::swap(&mut natural_width, &mut natural_height);
    }
    let mut width = f64::from(target_width);
    let mut height = width * f64::from(natural_height) / f64::from(natural_width);
    if !height.is_finite() || height <= 0.0 {
        return Err("PDF page dimensions overflowed".into());
    }
    let pixels = width * height;
    if pixels > MAX_PIXELS as f64 {
        let scale = (MAX_PIXELS as f64 / pixels).sqrt();
        width = (width * scale).floor().max(1.0);
        height = (height * scale).floor().max(1.0);
    }
    let width = width as u32;
    let height = height.round().max(1.0) as u32;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("PDF bitmap exceeds pixel budget".into());
    }
    // alpha=1 consistently yields a 4-byte BGRA bitmap on PDFium platforms.
    let bitmap = unsafe { (pdfium.bitmap_create)(width as c_int, height as c_int, 1) };
    if bitmap.is_null() {
        return Err("PDF bitmap allocation failed".into());
    }
    let result = bitmap_to_png(pdfium, bitmap, page, width, height, rotation, output);
    unsafe { (pdfium.bitmap_destroy)(bitmap) };
    result.map(|()| (width, height))
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn bitmap_to_png(pdfium: &Pdfium, bitmap: *mut c_void, page: *mut c_void, width: u32, height: u32, rotation: c_int, output: &Path) -> Result<(), String> {
    unsafe {
        (pdfium.bitmap_fill)(bitmap, 0, 0, width as c_int, height as c_int, 0xFFFF_FFFF);
        // Rendering annotations is safe; form actions/JS are never initialized.
        (pdfium.render_bitmap)(bitmap, page, 0, 0, width as c_int, height as c_int, rotation, 0x1);
    }
    let stride = unsafe { (pdfium.bitmap_stride)(bitmap) };
    let ptr = unsafe { (pdfium.bitmap_buffer)(bitmap) }.cast::<u8>();
    let min_stride = width.checked_mul(4).ok_or("PDF row overflow")? as usize;
    if ptr.is_null() || stride < 0 || (stride as usize) < min_stride || (stride as usize) > min_stride + 4096 {
        return Err("PDFium bitmap layout is invalid".into());
    }
    let mut rgba = Vec::with_capacity(min_stride * height as usize);
    for y in 0..height as usize {
        // SAFETY: Valid bitmap remains allocated through the conversion.
        let row = unsafe { std::slice::from_raw_parts(ptr.add(y * stride as usize), min_stride) };
        for bgra in row.chunks_exact(4) {
            rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    image::save_buffer(output, &rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| format!("PDF image could not be saved: {e}"))
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
struct Pdfium {
    module: *mut c_void,
    destroy: unsafe extern "C" fn(),
    load_document: unsafe extern "C" fn(*const c_void, c_int, *const c_char) -> *mut c_void,
    close_document: unsafe extern "C" fn(*mut c_void),
    page_count: unsafe extern "C" fn(*mut c_void) -> c_int,
    load_page: unsafe extern "C" fn(*mut c_void, c_int) -> *mut c_void,
    close_page: unsafe extern "C" fn(*mut c_void),
    page_width: unsafe extern "C" fn(*mut c_void) -> f32,
    page_height: unsafe extern "C" fn(*mut c_void) -> f32,
    bitmap_create: unsafe extern "C" fn(c_int, c_int, c_int) -> *mut c_void,
    bitmap_destroy: unsafe extern "C" fn(*mut c_void),
    bitmap_fill: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int, u32),
    render_bitmap: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, c_int, c_int, c_int, c_int, c_int),
    bitmap_stride: unsafe extern "C" fn(*mut c_void) -> c_int,
    bitmap_buffer: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
impl Pdfium {
    fn open() -> Result<Self, String> {
        #[cfg(target_os = "windows")] let library = "pdfium.dll";
        #[cfg(target_os = "linux")] let library = "libpdfium.so";
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let sibling = exe.with_file_name(library);
        let file = if sibling.is_file() { sibling } else { exe.with_file_name("resources").join(library) };
        let module = platform::open(&file)?;
        let resolved = unsafe {
            (|| -> Result<Self, String> {
                let init: unsafe extern "C" fn() = symbol(module, b"FPDF_InitLibrary\0")?;
                let value = Self {
                    module,
                    destroy: symbol(module, b"FPDF_DestroyLibrary\0")?,
                    load_document: symbol(module, b"FPDF_LoadMemDocument\0")?,
                    close_document: symbol(module, b"FPDF_CloseDocument\0")?,
                    page_count: symbol(module, b"FPDF_GetPageCount\0")?,
                    load_page: symbol(module, b"FPDF_LoadPage\0")?,
                    close_page: symbol(module, b"FPDF_ClosePage\0")?,
                    page_width: symbol(module, b"FPDF_GetPageWidthF\0")?,
                    page_height: symbol(module, b"FPDF_GetPageHeightF\0")?,
                    bitmap_create: symbol(module, b"FPDFBitmap_Create\0")?,
                    bitmap_destroy: symbol(module, b"FPDFBitmap_Destroy\0")?,
                    bitmap_fill: symbol(module, b"FPDFBitmap_FillRect\0")?,
                    render_bitmap: symbol(module, b"FPDF_RenderPageBitmap\0")?,
                    bitmap_stride: symbol(module, b"FPDFBitmap_GetStride\0")?,
                    bitmap_buffer: symbol(module, b"FPDFBitmap_GetBuffer\0")?,
                };
                init();
                Ok(value)
            })()
        };
        if resolved.is_err() {
            platform::close(module);
        }
        resolved
    }
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
impl Drop for Pdfium {
    fn drop(&mut self) {
        unsafe { (self.destroy)() };
        platform::close(self.module);
    }
}
#[cfg(any(target_os = "windows", target_os = "linux"))]
unsafe fn symbol<T: Copy>(module: *mut c_void, name: &[u8]) -> Result<T, String> {
    let ptr = platform::symbol(module, name.as_ptr().cast());
    if ptr.is_null() || std::mem::size_of::<T>() != std::mem::size_of::<*const c_void>() {
        return Err(format!("PDFium export missing: {}", String::from_utf8_lossy(name)));
    }
    Ok(unsafe { std::mem::transmute_copy(&ptr) })
}
#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> c_int;
    }
    pub fn open(path: &Path) -> Result<*mut c_void, String> {
        let absolute = path.canonicalize().map_err(|_| "Bundled PDFium library missing")?;
        let name: Vec<u16> = absolute.as_os_str().encode_wide().chain(Some(0)).collect();
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() { Err("Cannot load bundled PDFium".into()) } else { Ok(module) }
    }
    pub fn symbol(module: *mut c_void, name: *const c_char) -> *mut c_void { unsafe { GetProcAddress(module, name) } }
    pub fn close(module: *mut c_void) { unsafe { FreeLibrary(module) }; }
}
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::os::unix::ffi::OsStrExt;
    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dlclose(module: *mut c_void) -> c_int;
    }
    pub fn open(path: &Path) -> Result<*mut c_void, String> {
        let absolute = path.canonicalize().map_err(|_| "Bundled PDFium library missing")?;
        let mut name = absolute.as_os_str().as_bytes().to_vec();
        name.push(0);
        let module = unsafe { dlopen(name.as_ptr().cast(), 2) };
        if module.is_null() { Err("Cannot load bundled PDFium".into()) } else { Ok(module) }
    }
    pub fn symbol(module: *mut c_void, name: *const c_char) -> *mut c_void { unsafe { dlsym(module, name) } }
    pub fn close(module: *mut c_void) { unsafe { dlclose(module) }; }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pdf_worker_requires_explicit_invocation() {
        let args: Vec<std::ffi::OsString> = vec!["Akula".into(), "--not-pdf-worker".into()];
        assert_eq!(args[1], std::ffi::OsString::from("--not-pdf-worker"));
        assert!(MAX_INPUT <= 256 * 1024 * 1024);
    }
    #[test]
    fn rejects_too_large_raster_before_io() {
        let args = ["akula", "--pdf-worker", "fake.pdf", "0", "65000", "0", "x.png", "x.status"]
            .map(std::ffi::OsString::from);
        assert!(process(&args).is_err());
    }
}