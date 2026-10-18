//! Strict, bounded macOS updater TAR unpacking into a new empty staging tree.
//! This is a cooperating-process safety boundary, not hostile same-UID isolation.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

const BLOCK: usize = 512;
const MAX_ENTRIES: usize = 40_000;
const MAX_NAME: usize = 4096;
const MAX_DEPTH: usize = 64;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_EXT: u64 = 16 * 1024;
const MAX_TRAILER: usize = 1024 * 1024;

fn error(message: impl Into<String>) -> String {
    message.into()
}

fn octal(raw: &[u8], field: &str) -> Result<u64, String> {
    if raw.first().is_some_and(|b| b & 0x80 != 0) {
        return Err(error(format!("unsupported base-256 TAR {field}")));
    }
    let bytes = raw
        .iter()
        .copied()
        .skip_while(|b| *b == 0 || *b == b' ')
        .collect::<Vec<_>>();
    let end = bytes
        .iter()
        .position(|b| *b == 0 || *b == b' ')
        .unwrap_or(bytes.len());
    if bytes[end..].iter().any(|b| *b != 0 && *b != b' ') {
        return Err(error(format!("invalid TAR {field} terminator")));
    }
    if end == 0 {
        return Err(error(format!("missing TAR {field}")));
    }
    bytes[..end].iter().try_fold(0u64, |value, b| {
        if !(b'0'..=b'7').contains(b) {
            return Err(error(format!("invalid octal TAR {field}")));
        }
        value
            .checked_mul(8)
            .and_then(|v| v.checked_add((b - b'0') as u64))
            .ok_or_else(|| error(format!("overflowing TAR {field}")))
    })
}

fn header_text(raw: &[u8], field: &str) -> Result<String, String> {
    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
    let value = std::str::from_utf8(&raw[..end])
        .map_err(|_| error(format!("invalid UTF-8 in TAR {field}")))?;
    Ok(value.to_owned())
}

fn checked_header(header: &[u8; BLOCK]) -> Result<(), String> {
    let expected = octal(&header[148..156], "checksum")?;
    let actual: u64 = header
        .iter()
        .enumerate()
        .map(|(index, byte)| if (148..156).contains(&index) { 32 } else { *byte as u64 })
        .sum();
    if expected != actual {
        return Err(error("invalid TAR header checksum"));
    }
    Ok(())
}

fn next_header(reader: &mut impl Read) -> Result<[u8; BLOCK], String> {
    let mut block = [0u8; BLOCK];
    // Unlike read_exact().is_err(), this distinguishes a missing header
    // from a normal two-block TAR end marker and fails on a partial header.
    if reader.read(&mut block[..1]).map_err(|e| e.to_string())? == 0 {
        return Err(error("TAR ended before two zero terminator blocks"));
    }
    reader
        .read_exact(&mut block[1..])
        .map_err(|_| error("truncated TAR header"))?;
    Ok(block)
}

fn skip_padding(reader: &mut impl Read, len: u64) -> Result<(), String> {
    let padding = (BLOCK as u64 - (len % BLOCK as u64)) % BLOCK as u64;
    let mut bytes = [0u8; BLOCK];
    reader
        .read_exact(&mut bytes[..padding as usize])
        .map_err(|_| error("truncated TAR entry padding"))
}

fn extension(reader: &mut impl Read, size: u64) -> Result<Vec<u8>, String> {
    if size == 0 || size > MAX_EXT {
        return Err(error("invalid or oversized TAR extension"));
    }
    let mut payload = vec![0u8; size as usize];
    reader
        .read_exact(&mut payload)
        .map_err(|_| error("truncated TAR extension"))?;
    skip_padding(reader, size)?;
    Ok(payload)
}

fn long_text(payload: &[u8]) -> Result<String, String> {
    let end = payload.iter().position(|b| *b == 0).unwrap_or(payload.len());
    let value = std::str::from_utf8(&payload[..end])
        .map_err(|_| error("invalid GNU TAR extended filename"))?
        .trim_end_matches('\n');
    if value.is_empty() || value.len() > MAX_NAME {
        return Err(error("empty or oversized GNU TAR filename"));
    }
    Ok(value.to_owned())
}

fn pax(payload: &[u8]) -> Result<(Option<String>, Option<String>), String> {
    let mut pos = 0;
    let (mut path, mut linkpath) = (None, None);
    while pos < payload.len() {
        let remaining = &payload[pos..];
        let space = remaining.iter().position(|b| *b == b' ').ok_or("invalid PAX length")?;
        let count = std::str::from_utf8(&remaining[..space])
            .map_err(|_| error("invalid PAX length"))?
            .parse::<usize>()
            .map_err(|_| error("invalid PAX length"))?;
        if count <= space + 3 || count > remaining.len() {
            return Err(error("invalid PAX record bounds"));
        }
        let record = &remaining[space + 1..count];
        if !record.ends_with(b"\n") {
            return Err(error("unterminated PAX record"));
        }
        let pair = &record[..record.len() - 1];
        let equals = pair.iter().position(|b| *b == b'=').ok_or("invalid PAX field")?;
        let key = std::str::from_utf8(&pair[..equals]).map_err(|_| "invalid PAX key")?;
        let value = std::str::from_utf8(&pair[equals + 1..])
            .map_err(|_| error("invalid PAX value"))?
            .to_owned();
        match key {
            "path" if path.is_none() => path = Some(value),
            "linkpath" if linkpath.is_none() => linkpath = Some(value),
            "path" | "linkpath" => return Err(error("duplicate PAX path override")),
            "size" | "SCHILY.realsize" => return Err(error("PAX size override is forbidden")),
            _ if key.starts_with("GNU.sparse") => return Err(error("sparse TAR is forbidden")),
            _ => {} // Other non-layout PAX metadata is intentionally ignored.
        }
        pos += count;
    }
    Ok((path, linkpath))
}

fn relative_name(name: &str) -> Result<Option<String>, String> {
    if name == "Aquilum.app" || name == "Aquilum.app/" {
        return Ok(None);
    }
    let rest = name
        .strip_prefix("Aquilum.app/")
        .ok_or_else(|| error("TAR entry outside Aquilum.app"))?
        .trim_end_matches('/');
    if rest.is_empty() || rest.len() > MAX_NAME || rest.contains('\\') {
        return Err(error("unsafe TAR path"));
    }
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() > MAX_DEPTH
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.bytes().any(|b| b < 32 || b == 127)
        })
    {
        return Err(error("unsafe TAR path components"));
    }
    Ok(Some(parts.join("/")))
}

fn safe_link(name: &str, target: &str) -> Result<(), String> {
    if target.is_empty()
        || target.starts_with('/')
        || target.contains('\\')
        || target.bytes().any(|b| b == 0 || b < 32 || b == 127)
        || target.len() > MAX_NAME
    {
        return Err(error(format!("unsafe TAR symlink target at {name}")));
    }
    let mut depth = name.split('/').count() - 1;
    for part in target.split('/') {
        match part {
            "" => return Err(error("empty TAR symlink target component")),
            "." => {}
            ".." if depth > 0 => depth -= 1,
            ".." => return Err(error("TAR symlink escapes app bundle")),
            _ => depth += 1,
        }
        if depth > MAX_DEPTH {
            return Err(error("TAR symlink target too deep"));
        }
    }
    Ok(())
}

fn register(
    relative: &str,
    names: &mut HashMap<String, String>,
    explicit: &mut HashSet<String>,
) -> Result<(), String> {
    if !explicit.insert(relative.to_owned()) {
        return Err(error("duplicate TAR entry"));
    }
    let mut prefix = String::new();
    for part in relative.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        let key = prefix.to_lowercase();
        if let Some(existing) = names.get(&key) {
            if existing != &prefix {
                return Err(error("case-colliding TAR path"));
            }
        } else {
            names.insert(key, prefix.clone());
        }
    }
    Ok(())
}

fn ensure_parent(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let parts: Vec<_> = relative.split('/').collect();
    let mut current = root.to_path_buf();
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(stat) if stat.file_type().is_dir() && !stat.file_type().is_symlink() => {}
            Ok(_) => return Err(error("TAR parent is not a real directory")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(root.join(relative))
}

fn trailing_zeros(reader: &mut impl Read) -> Result<(), String> {
    let second = next_header(reader)?;
    if second.iter().any(|b| *b != 0) {
        return Err(error("TAR is missing its second zero block"));
    }
    let mut trailing = 0usize;
    let mut buffer = [0u8; 8192];
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        trailing += n;
        if trailing > MAX_TRAILER || buffer[..n].iter().any(|b| *b != 0) {
            return Err(error("nonzero or excessive TAR trailer"));
        }
    }
    Ok(())
}

/// Only accepts a complete, bounded archive whose every path stays in
/// Aquilum.app. The caller must own an empty, dedicated staging directory.
pub(super) fn unpack(mut reader: impl Read, root: &Path) -> Result<(), String> {
    let root_metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(error("updater staging root must be a real directory"));
    }
    if fs::read_dir(root).map_err(|e| e.to_string())?.next().is_some() {
        return Err(error("updater staging root must be empty"));
    }

    let mut headers = 0usize;
    let mut total = 0u64;
    let mut names = HashMap::new();
    let mut explicit = HashSet::new();
    let mut dirs = BTreeMap::<PathBuf, u32>::new();
    let app_root = root.join("Aquilum.app");
    let (mut path_override, mut link_override) = (None, None);
    let mut has_root = false;
    loop {
        let header = next_header(&mut reader)?;
        if header.iter().all(|b| *b == 0) {
            if path_override.is_some() || link_override.is_some() {
                return Err(error("unconsumed TAR path extension"));
            }
            trailing_zeros(&mut reader)?;
            break;
        }
        checked_header(&header)?;
        headers += 1;
        if headers > MAX_ENTRIES {
            return Err(error("too many TAR entries"));
        }
        let size = octal(&header[124..136], "size")?;
        let raw_mode = octal(&header[100..108], "mode")?;
        if raw_mode > 0o777 {
            return Err(error("privileged or unsupported TAR permission bits"));
        }
        let mode = raw_mode as u32;
        let kind = header[156];
        match kind {
            b'L' | b'K' => {
                let value = long_text(&extension(&mut reader, size)?)?;
                let slot = if kind == b'L' { &mut path_override } else { &mut link_override };
                if slot.replace(value).is_some() {
                    return Err(error("ambiguous GNU TAR path extension"));
                }
                continue;
            }
            b'x' => {
                let (path, link) = pax(&extension(&mut reader, size)?)?;
                if let Some(path) = path {
                    if path_override.replace(path).is_some() {
                        return Err(error("ambiguous PAX filename"));
                    }
                }
                if let Some(link) = link {
                    if link_override.replace(link).is_some() {
                        return Err(error("ambiguous PAX linkpath"));
                    }
                }
                continue;
            }
            b'g' => {
                let (path, link) = pax(&extension(&mut reader, size)?)?;
                if path.is_some() || link.is_some() {
                    return Err(error("global PAX path overrides are unsupported"));
                }
                continue;
            }
            b'0' | 0 | b'5' | b'2' => {}
            _ => return Err(error("unsupported TAR member type")),
        }

        let prefix = header_text(&header[345..500], "prefix")?;
        let name = header_text(&header[0..100], "filename")?;
        let header_name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let actual_name = path_override.take().unwrap_or(header_name);
        let link = link_override
            .take()
            .unwrap_or(header_text(&header[157..257], "linkname")?);

        let relative = relative_name(&actual_name)?;
        let Some(relative) = relative else {
            if kind != b'5' || size != 0 || has_root {
                return Err(error("invalid or repeated TAR bundle root"));
            }
            has_root = true;
            fs::create_dir(&app_root).map_err(|e| e.to_string())?;
            dirs.insert(app_root.clone(), mode);
            continue;
        };
        if !has_root {
            return Err(error("TAR app entries must follow the bundle root"));
        }
        register(&relative, &mut names, &mut explicit)?;
        let target = ensure_parent(&app_root, &relative)?;
        match kind {
            b'5' => {
                if size != 0 {
                    return Err(error("TAR directory has nonzero payload"));
                }
                match fs::symlink_metadata(&target) {
                    Ok(stat) if stat.is_dir() && !stat.file_type().is_symlink() => {}
                    Ok(_) => return Err(error("TAR directory conflicts with existing entry")),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        fs::create_dir(&target).map_err(|e| e.to_string())?;
                    }
                    Err(e) => return Err(e.to_string()),
                }
                dirs.insert(target, mode);
            }
            b'2' => {
                if size != 0 {
                    return Err(error("TAR symlink cannot have file payload"));
                }
                safe_link(&relative, &link)?;
                symlink(&link, &target).map_err(|e| e.to_string())?;
            }
            _ => {
                if size > MAX_FILE {
                    return Err(error("oversized TAR file"));
                }
                total = total
                    .checked_add(size)
                    .ok_or_else(|| error("TAR size overflow"))?;
                if total > MAX_TOTAL {
                    return Err(error("TAR archive exceeds extraction budget"));
                }
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|e| e.to_string())?;
                let copied = io::copy(&mut (&mut reader).take(size), &mut file)
                    .map_err(|e| e.to_string())?;
                if copied != size {
                    return Err(error("truncated TAR file body"));
                }
                file.flush().map_err(|e| e.to_string())?;
                file.set_permissions(fs::Permissions::from_mode(mode))
                    .map_err(|e| e.to_string())?;
            }
        }
        skip_padding(&mut reader, size)?;
    }
    if !has_root {
        return Err(error("missing Aquilum.app TAR root directory"));
    }
    for essential in ["Contents/Info.plist", "Contents/MacOS/aquilum-app"] {
        if !explicit.contains(essential) {
            return Err(error(format!("missing essential macOS bundle member: {essential}")));
        }
        let metadata = fs::symlink_metadata(app_root.join(essential))
            .map_err(|e| e.to_string())?;
        if !metadata.file_type().is_file() {
            return Err(error(format!("essential macOS bundle member is not a regular file: {essential}")));
        }
    }
    // Delay restrictive directory permissions until all children exist.
    let mut dirs: Vec<_> = dirs.into_iter().collect();
    dirs.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    for (path, mode) in dirs {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::{unpack, MAX_FILE};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Stage(PathBuf);

    impl Stage {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "akula-tar-security-test-{}-{id}", std::process::id()
            ));
            fs::create_dir(&path).expect("test staging directory");
            Self(path)
        }
    }

    impl Drop for Stage {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn octal(block: &mut [u8], number: u64) {
        let digits = format!("{number:0width$o}", width = block.len() - 1);
        assert!(digits.len() < block.len());
        block.fill(0);
        block[..digits.len()].copy_from_slice(digits.as_bytes());
    }

    fn header(name: &str, kind: u8, size: u64, mode: u32, target: &str) -> [u8; 512] {
        let mut h = [0u8; 512];
        assert!(name.len() <= 100);
        assert!(target.len() <= 100);
        h[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut h[100..108], mode as u64);
        octal(&mut h[108..116], 0);
        octal(&mut h[116..124], 0);
        octal(&mut h[124..136], size);
        octal(&mut h[136..148], 0);
        h[156] = kind;
        h[157..157 + target.len()].copy_from_slice(target.as_bytes());
        h[257..263].copy_from_slice(b"ustar\0");
        h[148..156].fill(b' ');
        let sum: u64 = h.iter().map(|b| *b as u64).sum();
        let checksum = format!("{sum:06o}\0 ");
        assert_eq!(checksum.len(), 8);
        h[148..156].copy_from_slice(checksum.as_bytes());
        h
    }

    fn push(out: &mut Vec<u8>, name: &str, kind: u8, body: &[u8], target: &str) {
        let mode = if kind == b'5' { 0o755 } else { 0o644 };
        let size = if matches!(kind, b'0' | 0 | b'L' | b'K' | b'x') {
            body.len() as u64
        } else {
            0
        };
        out.extend_from_slice(&header(name, kind, size, mode, target));
        out.extend_from_slice(body);
        out.extend(std::iter::repeat_n(0u8, (512 - body.len() % 512) % 512));
    }

    fn root(out: &mut Vec<u8>) {
        push(out, "Aquilum.app/", b'5', b"", "");
        push(out, "Aquilum.app/Contents/", b'5', b"", "");
        push(out, "Aquilum.app/Contents/MacOS/", b'5', b"", "");
    }

    fn files(out: &mut Vec<u8>) {
        push(out, "Aquilum.app/Contents/Info.plist", b'0', b"plist", "");
        push(
            out,
            "Aquilum.app/Contents/MacOS/aquilum-app",
            b'0',
            b"binary bytes",
            "",
        );
    }

    fn finish(out: &mut Vec<u8>) {
        out.extend(std::iter::repeat_n(0u8, 1024));
    }

    fn good() -> Vec<u8> {
        let mut tar = Vec::new();
        root(&mut tar);
        files(&mut tar);
        finish(&mut tar);
        tar
    }

    fn rejects(tar: Vec<u8>, expected: &str) {
        let stage = Stage::new();
        let problem = unpack(Cursor::new(tar), &stage.0).expect_err("must refuse archive");
        assert!(
            problem.contains(expected),
            "expected {expected:?}, received {problem:?}"
        );
    }

    #[test]
    fn writes_only_the_expected_bundle() {
        let stage = Stage::new();
        unpack(Cursor::new(good()), &stage.0).unwrap();
        assert_eq!(
            fs::read(stage.0.join("Aquilum.app/Contents/MacOS/aquilum-app")).unwrap(),
            b"binary bytes"
        );
        assert!(!stage.0.join("Contents").exists());
    }

    #[test]
    fn rejects_a_partial_header() {
        rejects(good()[..32].to_vec(), "truncated TAR header");
    }

    #[test]
    fn rejects_a_missing_end_marker() {
        let mut tar = good();
        tar.truncate(tar.len() - 1024);
        rejects(tar, "before two zero terminator");
    }

    #[test]
    fn rejects_truncated_file_data() {
        let mut tar = Vec::new();
        root(&mut tar);
        tar.extend_from_slice(&header(
            "Aquilum.app/Contents/Info.plist",
            b'0',
            400,
            0o644,
            "",
        ));
        tar.extend_from_slice(b"partial");
        rejects(tar, "truncated TAR file body");
    }

    #[test]
    fn rejects_invalid_header_checksum() {
        let mut tar = good();
        tar[100] ^= 1;
        rejects(tar, "checksum");
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/../outside", b'0', b"bad", "");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "unsafe TAR path");
    }

    #[test]
    fn rejects_out_of_bundle_file() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Another.app/evil", b'0', b"bad", "");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "outside Aquilum.app");
    }

    #[test]
    fn rejects_symlink_escape() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/escape", b'2', b"", "../../outside");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "symlink escapes");
    }

    #[test]
    fn rejects_writing_through_symlink_parent() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/alias", b'2', b"", "Contents");
        push(&mut tar, "Aquilum.app/alias/escape", b'0', b"bad", "");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "not a real directory");
    }

    #[test]
    fn rejects_duplicate_entry() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/Contents/Info.plist", b'0', b"a", "");
        push(&mut tar, "Aquilum.app/Contents/Info.plist", b'0', b"b", "");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "duplicate TAR entry");
    }

    #[test]
    fn rejects_case_collisions() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/contents/Info.plist", b'0', b"bad", "");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "case-colliding");
    }

    #[test]
    fn rejects_hard_links() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(
            &mut tar,
            "Aquilum.app/hardlink",
            b'1',
            b"",
            "Aquilum.app/Contents/Info.plist",
        );
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "unsupported TAR member");
    }

    #[test]
    fn rejects_huge_file_length_before_reading() {
        let mut tar = Vec::new();
        root(&mut tar);
        tar.extend_from_slice(&header(
            "Aquilum.app/Contents/Info.plist",
            b'0',
            MAX_FILE + 1,
            0o644,
            "",
        ));
        rejects(tar, "oversized TAR file");
    }

    #[test]
    fn rejects_nonzero_trailing_data() {
        let mut tar = good();
        tar.extend_from_slice(b"smuggled bytes");
        rejects(tar, "nonzero or excessive TAR trailer");
    }

    #[test]
    fn rejects_staging_directory_with_existing_contents() {
        let stage = Stage::new();
        fs::write(stage.0.join("already-present"), b"do not overwrite").unwrap();
        let problem = unpack(Cursor::new(good()), &stage.0).unwrap_err();
        assert!(problem.contains("must be empty"));
        assert_eq!(
            fs::read(stage.0.join("already-present")).unwrap(),
            b"do not overwrite"
        );
    }

    #[test]
    fn accepts_safe_internal_symlink() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/Contents/Copy", b'2', b"", "Info.plist");
        files(&mut tar);
        finish(&mut tar);
        let stage = Stage::new();
        unpack(Cursor::new(tar), &stage.0).unwrap();
        assert_eq!(
            fs::read_link(stage.0.join("Aquilum.app/Contents/Copy")).unwrap(),
            std::path::Path::new("Info.plist")
        );
    }

    #[test]
    fn accepts_bounded_gnu_long_filename() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/Contents/Resources/", b'5', b"", "");
        let name = format!("Aquilum.app/Contents/Resources/{}", "a".repeat(150));
        let mut payload = name.as_bytes().to_vec();
        payload.push(0);
        push(&mut tar, "././@LongLink", b'L', &payload, "");
        push(
            &mut tar,
            "Aquilum.app/Contents/Resources/placeholder",
            b'0',
            b"long name file",
            "",
        );
        files(&mut tar);
        finish(&mut tar);
        let stage = Stage::new();
        unpack(Cursor::new(tar), &stage.0).unwrap();
        assert_eq!(fs::read(stage.0.join(name)).unwrap(), b"long name file");
    }

    fn pax_record(data: &str) -> Vec<u8> {
        let mut len = data.len() + 2;
        loop {
            let result = format!("{len} {data}");
            if result.len() == len {
                return result.into_bytes();
            }
            len = result.len();
        }
    }

    #[test]
    fn accepts_bounded_pax_path_override() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/Contents/Resources/", b'5', b"", "");
        let record = pax_record("path=Aquilum.app/Contents/Resources/pax.txt\n");
        push(&mut tar, "././@PaxHeader", b'x', &record, "");
        push(
            &mut tar,
            "Aquilum.app/Contents/Resources/placeholder",
            b'0',
            b"pax",
            "",
        );
        files(&mut tar);
        finish(&mut tar);
        let stage = Stage::new();
        unpack(Cursor::new(tar), &stage.0).unwrap();
        assert_eq!(
            fs::read(stage.0.join("Aquilum.app/Contents/Resources/pax.txt")).unwrap(),
            b"pax"
        );
    }

    #[test]
    fn rejects_pax_size_override() {
        let mut tar = Vec::new();
        root(&mut tar);
        let record = pax_record("size=999999999999\n");
        push(&mut tar, "././@PaxHeader", b'x', &record, "");
        push(&mut tar, "Aquilum.app/Contents/fake", b'0', b"no", "");
        files(&mut tar);
        finish(&mut tar);
        rejects(tar, "PAX size override is forbidden");
    }

    #[test]
    fn rejects_missing_critical_bundle_contents() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/Contents/Info.plist", b'0', b"x", "");
        finish(&mut tar);
        rejects(tar, "missing essential macOS bundle member");
    }

    #[test]
    fn rejects_invalid_symlink_as_essential_binary() {
        let mut tar = Vec::new();
        root(&mut tar);
        push(&mut tar, "Aquilum.app/Contents/Info.plist", b'0', b"x", "");
        push(
            &mut tar,
            "Aquilum.app/Contents/MacOS/aquilum-app",
            b'2',
            b"",
            "../Info.plist",
        );
        finish(&mut tar);
        rejects(tar, "not a regular file");
    }
}
