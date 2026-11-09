use super::*;
use std::os::windows::fs::MetadataExt;
use std::process::Command;

#[test]
fn excludes_ntfs_junction_even_when_it_looks_like_a_directory() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(workspace.join("safe.md"), "safe").unwrap();
    fs::write(outside.join("private.md"), "private").unwrap();

    // /J is supported for ordinary Windows users; no symlink privilege required.
    let junction = workspace.join("linked-outside");
    let command = format!("mklink /J \"{}\" \"{}\"", junction.display(), outside.display());
    let output = Command::new("cmd.exe")
        .args(["/C", &command])
        .output()
        .expect("start Windows junction creation");
    assert!(
        output.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_ne!(
        fs::symlink_metadata(&junction).unwrap().file_attributes() & 0x0000_0400,
        0,
        "fixture must actually be an NTFS reparse point",
    );

    let entries = read_directory_impl(&workspace).unwrap();
    let names: Vec<_> = entries.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(names, ["safe"], "junction may escape the workspace");
}
