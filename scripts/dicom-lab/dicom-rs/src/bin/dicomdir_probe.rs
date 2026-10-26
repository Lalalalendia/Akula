//! Read-only flat DICOMDIR reference inspection. Not full pointer/offset traversal.
use dicom_core::Tag;
use dicom_object::open_file;
use std::{env, error::Error, io::{Error as IoError, ErrorKind}, path::{Path, PathBuf}};

const RECORDS: Tag = Tag(0x0004, 0x1220);
const RECORD_TYPE: Tag = Tag(0x0004, 0x1430);
const FILE_ID: Tag = Tag(0x0004, 0x1500);

fn invalid(message: impl Into<String>) -> IoError {
    IoError::new(ErrorKind::InvalidData, message.into())
}

fn parse_referenced_path(root: &Path, ids: &[String]) -> Result<PathBuf, IoError> {
    let mut p = root.to_path_buf();
    let mut segments = 0;
    for field in ids {
        // Referenced File ID can be one CS string with a backslash-separated
        // value, or be represented as multiple components by the parser.
        for piece in field.split('\\') {
            let name = piece.trim_matches(|c: char| c == '\0' || c == ' ');
            if name.is_empty() || name == "." || name == ".." ||
                name.len() > 8 ||
                !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                return Err(invalid(format!("invalid DICOM File ID component: {:?}", piece)));
            }
            p.push(name);
            segments += 1;
        }
    }
    if segments == 0 || segments > 8 {
        return Err(invalid("invalid DICOM File ID component count"));
    }
    Ok(p)
}

fn run(path: &Path, expected_missing: usize) -> Result<(), Box<dyn Error>> {
    let root = path.parent().ok_or_else(|| invalid("DICOMDIR has no parent"))?.canonicalize()?;
    let index = open_file(path)?;
    let sequence = index.element(RECORDS)?;
    let records = sequence.items().ok_or_else(|| invalid("Directory Record Sequence is not SQ"))?;
    let mut refs = 0_usize;
    let mut present = 0_usize;
    let mut missing = 0_usize;
    for record in records {
        let typ = record.element(RECORD_TYPE)?.to_str()?;
        if typ.trim_matches(|c: char| c == '\0' || c == ' ') != "IMAGE" {
            continue;
        }
        refs += 1;
        let values = record.element(FILE_ID)?.to_multi_str()?;
        let p = parse_referenced_path(&root, values.as_ref())?;
        if p.is_file() {
            let resolved = p.canonicalize()?;
            if !resolved.starts_with(&root) {
                return Err(invalid("DICOMDIR path resolves outside the volume").into());
            }
            // Check read-only access, without parsing/decompressing full pixel data.
            let _file = std::fs::File::open(resolved)?;
            present += 1;
        } else {
            missing += 1;
        }
    }
    println!("DICOMDIR_RUST records={} references={} present={} missing={}",
             records.len(), refs, present, missing);
    if refs != 3 || missing != expected_missing || present + missing != refs {
        return Err(invalid(format!("expected 3 refs / {} missing, observed {} refs / {} missing",
                                   expected_missing, refs, missing)).into());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: dicomdir_probe <DICOMDIR path> <expected missing image files>");
        std::process::exit(2);
    }
    let missing = args[2].parse::<usize>().unwrap_or(usize::MAX);
    if let Err(e) = run(Path::new(&args[1]), missing) {
        eprintln!("DICOMDIR_RUST_ERROR: {e}");
        std::process::exit(1);
    }
}
