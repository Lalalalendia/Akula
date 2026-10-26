#[path = "../../../aquilum-app/native/src/dicom.rs"]
mod dicom;
use std::path::Path;
fn next_event(worker: &dicom::DicomWorker) -> dicom::FrameUpdate {
    for _ in 0..200 {
        match worker.poll() {
            Ok(Some(update)) => return update,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(err) => panic!("async DICOM worker disconnected: {err}"),
        }
    }
    panic!("DICOM worker timeout");
}

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
    // Exercise the exact model used by the UI: off-thread open+decode and
    // sequential page navigation, without launching Masonry on the CI runner.
    let async_worker = dicom::DicomWorker::start(
        std::path::PathBuf::from(&vault),
        std::path::PathBuf::from(&study_path)
    );
    let first = next_event(&async_worker);
    assert!(first.error.is_none(), "{:?}", first.error);
    assert_eq!(first.frame.as_ref().unwrap().rgba.len(), 8 * 8 * 4);
    assert!(async_worker.request(dicom::Navigation::Frame(1)));
    let second = next_event(&async_worker);
    assert!(second.error.is_none(), "{:?}", second.error);
    assert!(second.label.contains("срез 2/3"));
    println!("DICOM_NATIVE_ASYNC_OK initial=1 navigated=2 rgba_bytes=256");

}
