#[path = "../../../aquilum-app/native/src/dicom.rs"]
mod dicom;
use std::path::Path;
fn main() {
    let mut args = std::env::args().skip(1);
    let vault = args.next().expect("vault");
    let study_path = args.next().expect("DICOMDIR or image");
    let mut study = dicom::Study::open(Path::new(&vault), Path::new(&study_path))
        .expect("open study");
    let first = study.render().expect("render first");
    assert_eq!((first.width, first.height), (8,8));
    assert_eq!(first.rgba.len(), 8*8*4);
    let mut n = 1;
    while study.advance_frame(1) {
        let frame = study.render().expect("render next");
        assert_eq!((frame.width, frame.height), (8,8));
        n+=1;
    }
    println!("DICOM_NATIVE_CORE_OK frames={n} label={}", study.label());
}
