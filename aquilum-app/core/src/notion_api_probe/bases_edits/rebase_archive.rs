//! Immutable pre-decision backup of the exact Akula Bases JSON overlay.
//! A file created here is never overwritten or removed by source rebase.
//! No JSON Notion source file, application token or cloud provider is touched.
use super::DIR;
use crate::{app_core::Core, files::gate, history::Source};
use std::{fs, io::ErrorKind, path::Path};

const MAX_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;

fn archive_relative(source:&str, old_digest:&str, new_digest:&str, disk_hash:&str)->String{
    let mut hash=blake3::Hasher::new();
    hash.update(b"akula:bases:pre-rebase:v1");
    for part in [source,old_digest,new_digest,disk_hash] {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{DIR}/{source}-{}.akula-bases-backup.json",
        hash.finalize().to_hex())
}
/// Create a durable-in-application, exact-byte audit copy BEFORE destructive
/// local overlay changes. The File Gate does not certify crash/power durability
/// here; failures and existing unexpected copies fail closed.
pub(super) fn preserve_before_accept(
    core:&Core, root:&Path, source:&str, old_digest:&str, new_digest:&str,
    old_path:&Path, old_hash:&str,
)->Result<String,String>{
    let input=fs::symlink_metadata(old_path)
        .map_err(|_|"cannot inspect pre-rebase local overlay")?;
    if !input.is_file() || input.file_type().is_symlink()
        || input.len()>MAX_ARCHIVE_BYTES as u64 {
        return Err("unsafe or oversized source overlay; backup denied".into());
    }
    let bytes=fs::read(old_path)
        .map_err(|_|"cannot read source overlay for backup")?;
    if bytes.len()>MAX_ARCHIVE_BYTES
        || blake3::hash(&bytes).to_hex().to_string()!=old_hash {
        return Err("source overlay changed before pre-rebase backup".into());
    }
    let serialized=std::str::from_utf8(&bytes)
        .map_err(|_|"source overlay is not valid UTF-8")?;
    let relative=archive_relative(source,old_digest,new_digest,old_hash);
    let target=root.join(&relative);
    match fs::symlink_metadata(&target) {
        Ok(meta)=>{
            #[cfg(windows)]
            let reparse={
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x0000_0400 != 0
            };
            #[cfg(not(windows))]
            let reparse=false;
            if !meta.is_file() || meta.file_type().is_symlink()
                || reparse || meta.len()>MAX_ARCHIVE_BYTES as u64 {
                return Err("unsafe existing Akula Bases backup".into());
            }
        }
        Err(error) if error.kind()==ErrorKind::NotFound=>{
            gate::create(core,&target,serialized,Source::Me,
                Some("Akula Bases pre-rebase local archive"))
                .map_err(|_|"File Gate denied pre-rebase backup; local edits preserved")?;
        }
        Err(_)=>return Err("cannot inspect Akula Bases backup destination".into()),
    }
    let back=fs::read(&target).map_err(|_|"cannot verify pre-rebase backup")?;
    if back!=bytes || blake3::hash(&back).to_hex().to_string()!=old_hash {
        return Err("backup verification failed; do not change local overlay".into());
    }
    Ok(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_name_is_deterministic_and_bound_to_all_revisions(){
        let a=archive_relative("root","old","new","hash");
        assert_eq!(a,archive_relative("root","old","new","hash"));
        assert_ne!(a,archive_relative("root","old","new2","hash"));
        assert_ne!(a,archive_relative("root","old","new","hash2"));
        assert!(a.starts_with("Notion-import/root-"));
        assert!(a.ends_with(".akula-bases-backup.json"));
    }
}
