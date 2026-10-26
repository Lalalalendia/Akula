#[path = "../../../aquilum-app/native/src/dicom.rs"]
mod dicom;
use std::path::{Path, PathBuf};

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

fn verify_inversion(root: &Path) {
    let mono1 = dicom::Study::open(root, &root.join("MONO1.dcm")).expect("read MONOCHROME1")
        .render().expect("render MONOCHROME1");
    let mono2 = dicom::Study::open(root, &root.join("MONO2.dcm")).expect("read MONOCHROME2")
        .render().expect("render MONOCHROME2");
    assert_eq!((mono1.width, mono1.height), (8, 8));
    assert_eq!((mono2.width, mono2.height), (8, 8));
    for (first, second) in mono1.rgba.chunks_exact(4).zip(mono2.rgba.chunks_exact(4)) {
        for channel in 0..3 {
            assert_eq!(u16::from(first[channel]) + u16::from(second[channel]), 255,
                "MONOCHROME1 must invert MONOCHROME2 brightness");
        }
        assert_eq!(first[3], 255);
        assert_eq!(second[3], 255);
    }
    println!("DICOM_MONOCHROME1_INVERSION_OK pixels=64");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let vault = PathBuf::from(args.next().expect("vault"));
    let source = PathBuf::from(args.next().expect("DICOMDIR or image"));
    let filename = source.file_name().unwrap().to_string_lossy().into_owned();
    let signed = filename == "SIGNED16.dcm";
    let in_place = filename == "study.zip" || filename == "loose_folder";
    let original_zip = if filename == "study.zip" { Some(std::fs::read(&source).expect("ZIP")) } else { None };
    let dimensions = if signed { (16, 16) } else { (8, 8) };
    let expected_bytes = dimensions.0 as usize * dimensions.1 as usize * 4;

    let mut study = dicom::Study::open(&vault, &source).expect("open study");
    let first = study.render().expect("render first");
    assert_eq!((first.width, first.height), dimensions);
    assert_eq!(first.rgba.len(), expected_bytes);
    let mut frames = vec![first.rgba];
    while study.advance_frame(1) {
        let frame = study.render().expect("render next");
        assert_eq!((frame.width, frame.height), dimensions);
        assert_eq!(frame.rgba.len(), expected_bytes);
        frames.push(frame.rgba);
    }
    let count = frames.len();
    if signed {
        assert_eq!(count, 3);
        assert_ne!(frames[0], frames[1], "signed frame 0 and 1 must not collapse");
        assert_ne!(frames[1], frames[2], "signed frame 1 and 2 must not collapse");
        println!("DICOM_SIGNED16_VOI_OK frames={count} shape=16x16");
    } else if filename == "DICOMDIR" || in_place {
        assert_eq!(count, 3);
        if filename == "DICOMDIR" { verify_inversion(&vault); }
    }
    if let Some(before) = original_zip { assert_eq!(std::fs::read(&source).unwrap(), before, "ZIP was mutated"); }
    if in_place { println!("DICOM_INPLACE_OK source={filename} frames={count}"); }
    println!("DICOM_NATIVE_CORE_OK frames={count} label={}", study.label());

    let worker = dicom::DicomWorker::start(vault.clone(), source.clone());
    let first = next_event(&worker);
    assert!(first.error.is_none(), "{:?}", first.error);
    assert_eq!(first.frame.as_ref().unwrap().rgba.len(), expected_bytes);
    if count > 1 {
        assert!(worker.request(dicom::Navigation::Frame(1)));
        let second = next_event(&worker);
        assert!(second.error.is_none(), "{:?}", second.error);
        assert!(second.label.contains(&format!("срез 2/{count}")));
        assert_eq!(second.frame.unwrap().rgba.len(), expected_bytes);
    }
    println!("DICOM_NATIVE_ASYNC_OK frames={count} rgba_bytes={expected_bytes}");
    if filename == "study.zip" {
        assert!(dicom::Study::open(&vault, &vault.join("unsafe.zip")).is_err());
        println!("DICOM_UNSAFE_ZIP_REJECTED");
    }
}
