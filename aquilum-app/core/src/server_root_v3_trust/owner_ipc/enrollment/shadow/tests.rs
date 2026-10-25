use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

const PREFIX: &[u8] = b"root:!:20000:0:99999:7:::\nnobody:*:20000:0:99999:7:::\n";

fn records(password: &str) -> Vec<u8> {
    let mut data = PREFIX.to_vec();
    data.extend_from_slice(
        format!("{SERVICE_ACCOUNT}:{password}:20000:0:99999:7:::\n").as_bytes(),
    );
    data
}

#[test]
fn locked_shadow_accounts_accept_standard_linux_lock_markers() {
    for token in ["!", "!!", "!$y$sealed-hash", "*", "*LK*", "!*"] {
        assert!(
            parse_locked_shadow(&records(token)).is_ok(),
            "lock token {token}"
        );
    }
    assert!(shadow_mode_allowed(0o000));
    assert!(shadow_mode_allowed(0o400));
    assert!(shadow_mode_allowed(0o600));
    assert!(shadow_mode_allowed(0o640));
}

#[test]
fn usable_password_missing_account_or_ambiguous_shadow_must_hold() {
    for token in ["", "$y$usable-hash", "$6$usable-hash", "password", "x", " !"] {
        assert!(
            parse_locked_shadow(&records(token)).is_err(),
            "unsafe password token {token:?}"
        );
    }
    assert!(parse_locked_shadow(PREFIX).is_err());
    let mut duplicate = records("!");
    duplicate.extend_from_slice(&records("!"));
    assert!(parse_locked_shadow(&duplicate).is_err());
    let mut extra_field = records("!");
    extra_field.extend_from_slice(b"broken:!:20000:0:99999:7::::\n");
    assert!(parse_locked_shadow(&extra_field).is_err());
    let mut invalid_utf8 = records("!");
    invalid_utf8.push(0xff);
    assert!(parse_locked_shadow(&invalid_utf8).is_err());
    for unsafe_mode in [0o002, 0o004, 0o6000, 0o660, 0o644, 0o666, 0o777] {
        assert!(!shadow_mode_allowed(unsafe_mode), "mode {unsafe_mode:o}");
    }
}

#[test]
fn shadow_inode_must_be_regular_root_owned_single_link_and_private() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("shadow");
    fs::write(&path, records("!")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let metadata = fs::metadata(&path).unwrap();
    if rustix::process::geteuid().as_raw() == 0 {
        assert!(root_shadow_ok(&metadata));
    } else {
        assert!(!root_shadow_ok(&metadata));
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!root_shadow_ok(&fs::metadata(&path).unwrap()));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&path, temporary.path().join("copy")).unwrap();
    assert!(!root_shadow_ok(&fs::metadata(&path).unwrap()));
    fs::remove_file(temporary.path().join("copy")).unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(!root_shadow_ok(&fs::metadata(&path).unwrap()));
}


#[test]
fn fixed_shadow_fd_accepts_only_regular_protected_inode_without_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadow");
    fs::write(&path, records("!")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let etc = File::open(dir.path()).unwrap();
    if rustix::process::geteuid().as_raw() == 0 {
        assert!(fixed_shadow_account_locked(&etc).is_ok());
    } else {
        assert!(fixed_shadow_account_locked(&etc).is_err());
    }
    fs::rename(&path, dir.path().join("saved-shadow")).unwrap();
    std::os::unix::fs::symlink("saved-shadow", &path).unwrap();
    assert!(fixed_shadow_account_locked(&etc).is_err());
    fs::remove_file(&path).unwrap();
    fs::rename(dir.path().join("saved-shadow"), &path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixed_shadow_account_locked(&etc).is_err());
}
