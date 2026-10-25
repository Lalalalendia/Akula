//! Research-only durable staging, separate from Akula's production File Gate.
//! Storage is capped by the manifest (16 MiB). All parts are verified before staging.
//! The caller chooses a PRIVATE staging directory and an explicit output file.

use crate::{Manifest, PartialDownload, sha};
use anyhow::{Context, Result, bail, ensure};
use std::{fs, io::Write, path::{Path, PathBuf}};

pub struct DurableDownload {
    dir: PathBuf,
    pub partial: PartialDownload,
}

fn check_regular_file(path: &Path) -> Result<()> {
    let md = fs::symlink_metadata(path)
        .with_context(|| format!("could not stat {}", path.display()))?;
    ensure!(md.file_type().is_file(), "not a regular file: {}", path.display());
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    if path.exists() {
        let md = fs::symlink_metadata(path)?;
        ensure!(md.file_type().is_dir(), "staging directory must not be a symlink or file");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            ensure!(md.permissions().mode() & 0o077 == 0, "staging directory must be private (chmod 700)");
        }
    } else {
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(path)?;
        }
        #[cfg(not(unix))] { fs::create_dir(path)?; }
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path).with_context(|| format!("create {}", path.display()))?;
    if let Err(e) = file.write_all(contents).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}

fn part_path(dir: &Path, index: usize) -> PathBuf {
    dir.join(format!("part-{index:04}.bin"))
}

impl DurableDownload {
    pub fn open(dir: &Path, manifest: Manifest) -> Result<Self> {
        manifest.validate()?;
        create_private_dir(dir)?;
        let manifest_path = dir.join("manifest.akm");
        let encoded = manifest.encode();
        if manifest_path.exists() {
            check_regular_file(&manifest_path)?;
            ensure!(fs::read(&manifest_path)? == encoded, "staging manifest does not match requested content");
        } else {
            write_new_file(&manifest_path, &encoded)?;
        }
        let mut partial = PartialDownload::new(manifest)?;
        for index in 0..partial.manifest.parts.len() {
            let p = part_path(dir, index);
            if fs::symlink_metadata(&p).is_err() { continue; }
            check_regular_file(&p)?;
            // A damaged or half-written old part must never count as complete.
            let bytes = fs::read(&p)?;
            if partial.add_part(index, bytes).is_err() {
                fs::remove_file(&p)?;
            }
        }
        Ok(Self { dir: dir.to_owned(), partial })
    }

    /// Atomically commit an already verified in-memory part, never an unchecked blob.
    pub fn commit_verified_part(&self, index: usize) -> Result<()> {
        let part = self.partial.verified_part(index).context("not a verified part")?;
        let dst = part_path(&self.dir, index);
        if dst.exists() {
            check_regular_file(&dst)?;
            ensure!(fs::read(&dst)? == part, "existing part differs");
            return Ok(());
        }
        let tmp = self.dir.join(format!("part-{index:04}.tmp"));
        if fs::symlink_metadata(&tmp).is_ok() {
            check_regular_file(&tmp)?;
            // A prior aborted write is not trusted; write the verified bytes again.
            fs::remove_file(&tmp)?;
        }
        write_new_file(&tmp, part)?;
        fs::rename(&tmp, &dst)?;
        Ok(())
    }

    pub fn first_missing(&self) -> Option<usize> {
        (0..self.partial.manifest.parts.len()).find(|&idx| self.partial.verified_part(idx).is_none())
    }

    /// Final output is only visible after the full hash is checked. Existing files are never replaced.
    pub fn finish_to(&self, output: &Path) -> Result<()> {
        ensure!(self.partial.is_complete(), "cannot publish incomplete book");
        if fs::symlink_metadata(output).is_ok() { bail!("output already exists (will not overwrite)"); }
        let filename = output.file_name().context("output must be a file path")?;
        let tempfile = output.with_file_name(format!(".{}.akula-tmp-{}", filename.to_string_lossy(), std::process::id()));
        if fs::symlink_metadata(&tempfile).is_ok() { bail!("leftover output temp file; refuse overwrite"); }
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tempfile)?;
        let result = (|| -> Result<()> {
            let mut hasher = blake3::Hasher::new();
            let mut total = 0u64;
            for index in 0..self.partial.manifest.parts.len() {
                let bytes = self.partial.verified_part(index).context("missing part")?;
                ensure!(bytes.len() == self.partial.manifest.part_len(index)?, "wrong part length");
                ensure!(sha(bytes) == self.partial.manifest.parts[index], "wrong part hash");
                file.write_all(bytes)?;
                hasher.update(bytes);
                total += bytes.len() as u64;
            }
            ensure!(total == self.partial.manifest.len, "wrong total length");
            ensure!(*hasher.finalize().as_bytes() == self.partial.manifest.digest, "whole-book hash mismatch");
            file.sync_all()?;
            drop(file);
            // hard_link, unlike rename on Unix, does NOT replace an existing target.
            fs::hard_link(&tempfile, output)?;
            fs::remove_file(&tempfile)?;
            Ok(())
        })();
        if result.is_err() { let _ = fs::remove_file(&tempfile); }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Book, CHUNK_SIZE};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tempdir() -> PathBuf {
        let ns = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("akula-lib-durable-{}-{ns}", std::process::id()));
        fs::create_dir(&p).unwrap();
        p
    }

    #[test]
    fn persists_verified_parts_and_restores_after_restart() -> Result<()> {
        let base = tempdir();
        let dir = base.join("stage");
        let book = Book::new((0..(CHUNK_SIZE * 2 + 19)).map(|i| i as u8).collect())?;
        let mut a = DurableDownload::open(&dir, book.manifest.clone())?;
        let first = book.part(0)?.to_vec();
        a.partial.add_part(0, first.clone())?;
        a.commit_verified_part(0)?;
        drop(a);
        let mut b = DurableDownload::open(&dir, book.manifest.clone())?;
        assert_eq!(b.partial.verified_count(), 1);
        assert_eq!(b.first_missing(), Some(1));
        for idx in 1..b.partial.manifest.parts.len() {
            b.partial.add_part(idx, book.part(idx)?.to_vec())?;
            b.commit_verified_part(idx)?;
        }
        let out = base.join("book.bin");
        b.finish_to(&out)?;
        assert_eq!(fs::read(&out)?, book.bytes.as_ref().clone());
        assert!(b.finish_to(&out).is_err());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn rejects_mixed_manifests_and_discards_bad_parts() -> Result<()> {
        let base = tempdir();
        let dir = base.join("stage");
        let book = Book::new(vec![8u8; CHUNK_SIZE + 5])?;
        let mut d = DurableDownload::open(&dir, book.manifest.clone())?;
        d.partial.add_part(0, book.part(0)?.to_vec())?;
        d.commit_verified_part(0)?;
        drop(d);
        fs::write(part_path(&dir, 0), b"broken")?;
        let restored = DurableDownload::open(&dir, book.manifest.clone())?;
        assert_eq!(restored.partial.verified_count(), 0);
        assert!(restored.finish_to(&base.join("bad.bin")).is_err());
        let other = Book::new(vec![9u8; CHUNK_SIZE + 5])?;
        assert!(DurableDownload::open(&dir, other.manifest).is_err());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn empty_file_can_be_published_without_chunks() -> Result<()> {
        let base = tempdir();
        let dir = base.join("stage");
        let book = Book::new(vec![])?;
        let d = DurableDownload::open(&dir, book.manifest)?;
        assert!(d.partial.is_complete());
        let out = base.join("empty.bin");
        d.finish_to(&out)?;
        assert_eq!(fs::read(out)?, Vec::<u8>::new());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn refuses_staging_symlink() -> Result<()> {
        use std::os::unix::fs::symlink;
        let base = tempdir();
        let good = base.join("good");
        fs::create_dir(&good)?;
        let link = base.join("link");
        symlink(&good, &link)?;
        let b = Book::new(vec![1u8; 8])?;
        assert!(DurableDownload::open(&link, b.manifest).is_err());
        fs::remove_dir_all(base)?;
        Ok(())
    }
}