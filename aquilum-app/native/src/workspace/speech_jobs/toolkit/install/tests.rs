use super::*;
use std::io::Write;

#[test]
fn archives_extract_only_pinned_files_and_reject_altered_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("bin/whisper-cli.exe", options).unwrap();
    zip.write_all(b"abc").unwrap();
    zip.start_file("../unwanted.dll", options).unwrap();
    zip.write_all(b"xyz").unwrap();
    zip.finish().unwrap();

    const PIN: Pin = Pin {
        name: "whisper-cli.exe",
        bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    };
    let good = Archive {
        prefix: "bin",
        folder: "clean",
        files: &[PIN],
        ..WHISPER
    };
    extract(&archive, temp.path(), &good).unwrap();
    assert_eq!(
        fs::read(temp.path().join("clean/whisper-cli.exe")).unwrap(),
        b"abc"
    );
    assert!(!temp.path().join("unwanted.dll").exists());

    let bad = Archive {
        folder: "modified",
        files: &[Pin {
            sha256: "wrong",
            ..PIN
        }],
        ..good
    };
    assert!(
        extract(&archive, temp.path(), &bad)
            .unwrap_err()
            .contains("file changed")
    );
}

#[test]
#[ignore = "requires local copies of the pinned Windows archives and models"]
fn official_speech_archives_produce_the_complete_verified_toolkit() {
    let temp = tempfile::tempdir().unwrap();
    let sources = std::path::PathBuf::from(
        std::env::var_os("AKULA_ASR_TEST_ARCHIVES").expect("pinned ASR archive directory"),
    );
    let mut progress = Vec::new();
    let install = stage_with(
        temp.path(),
        |percent| progress.push(percent),
        |name, transfer, dest, report| {
            let mut source = File::open(sources.join(name)).map_err(|error| error.to_string())?;
            transfer.copy_checked(&mut source, dest, report)
        },
    )
    .unwrap();
    for tool in Tool::ALL {
        checked(install.path(), tool).unwrap();
    }
    assert!(progress.windows(2).all(|step| step[0] <= step[1]));
    assert_eq!(progress.last().copied(), Some(99));
    fs::write(install.path().join("engine/unexpected.dll"), b"unexpected").unwrap();
    assert!(checked(install.path(), Tool::Whisper).is_err());
}
