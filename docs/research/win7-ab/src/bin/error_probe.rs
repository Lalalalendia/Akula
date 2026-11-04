//! Exercise the windows-result 0.4.1 ERROR path (full/slim variants).
fn main() {
    let err = windows_result::Error::new(
        windows_result::HRESULT(0x80004005u32 as i32),
        "akula-win7-probe",
    );
    println!("result-error-code {:?}", err.code());
    std::hint::black_box(err);
}
