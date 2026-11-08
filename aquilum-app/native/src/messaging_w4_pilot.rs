//! Explicit W3/W4 production-canary interface in the *main Akula executable*.
//! The reviewed WOW helper is compiled separately to avoid its rusqlite v0.32
//! conflicting with Akula Core's v0.40. The pilot has no implicit startup,
//! Vault writes, public listeners, trust-on-first-use, or shell invocation.

use std::{env, ffi::OsStr, fs, io::Read, path::{Path,PathBuf}, process::{Command,Stdio}};
use sha2::{Digest,Sha256};

const MAX_BINARY: u64 = 24 * 1024 * 1024;
const HASH: Option<&str> = option_env!("AKULA_W4_SIDECAR_SHA256");

#[derive(Debug,Clone,PartialEq,Eq)]
enum Action {
    Status, InitSender, InitRecipient,
    Listen { key: String, drop_ack: bool },
    Send { invite: String },
    Resend { invite: String, message: String },
}

fn hex(s:&str,n:usize)->bool { s.len()==n && s.bytes().all(|x| x.is_ascii_hexdigit()) }
fn parse(a:&[String])->Result<Action,&'static str>{
    match a {
        [v] if v=="status" => Ok(Action::Status),
        [v] if v=="init-sender" => Ok(Action::InitSender),
        [v] if v=="init-recipient" => Ok(Action::InitRecipient),
        [v,k] if (v=="listen"||v=="listen-drop-ack") && hex(k,64)
            => Ok(Action::Listen{key:k.clone(),drop_ack:v=="listen-drop-ack"}),
        [v,i] if v=="send-test" && hex(i,160) => Ok(Action::Send{invite:i.clone()}),
        [v,i,m] if v=="resend" && hex(i,160) && hex(m,32)
            => Ok(Action::Resend{invite:i.clone(),message:m.clone()}),
        _=>Err("команды: status | init-sender | init-recipient | listen <публичный_ключ_64> | listen-drop-ack <публичный_ключ_64> | send-test <приглашение_160> | resend <приглашение_160> <id_32>"),
    }
}
fn verify_executable(path:&Path, expected:Option<&str>)->Result<(),String>{
    let expected=expected.ok_or("у этой сборки Akula нет утверждённой суммы W4")?;
    if !hex(expected,64){return Err("неверная встроенная сумма W4".into());}
    let meta=fs::symlink_metadata(path).map_err(|_|"W4 отсутствует рядом с Akula".to_string())?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() ||
        meta.len()==0 || meta.len()>MAX_BINARY {return Err("неверный тип/размер W4".into());}
    let mut input=fs::File::open(path).map_err(|e|e.to_string())?;
    let mut digest=Sha256::new();
    let mut block=[0u8;65536];
    let mut count=0u64;
    loop {
        let n=input.read(&mut block).map_err(|e|e.to_string())?;
        if n==0 {break;}
        count+=n as u64;
        if count>MAX_BINARY {return Err("W4 превышает предел".into());}
        digest.update(&block[..n]);
    }
    let actual=digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    if actual!=expected.to_ascii_lowercase() {
        return Err("контрольная сумма W4 не совпадает; запуск запрещён".into());
    }
    Ok(())
}
fn helper()->Result<PathBuf,String>{
    let exe=env::current_exe().map_err(|e|e.to_string())?;
    let dir=exe.parent().ok_or("нет каталога Akula")?;
    // Standalone Akula stores a sibling helper. The AppImage copies the same
    // SHA-pinned ELF to usr/share so linuxdeploy cannot rewrite its PT_INTERP.
    let sibling=dir.join("akula-w4-lab");
    let path=match fs::symlink_metadata(&sibling) {
        Ok(_) => sibling, // if present, even an invalid symlink must fail closed
        Err(e) if e.kind()==std::io::ErrorKind::NotFound =>
            dir.parent().ok_or("нет каталога usr")?
                .join("share").join("akula-w4").join("akula-w4-lab"),
        Err(e)=>return Err(format!("проверка файла W4: {e}")),
    };
    verify_executable(&path,HASH)?;
    Ok(path)
}
fn private_root()->Result<PathBuf,String>{
    let base=crate::workspace::data_dir_or_default(None)?;
    fs::create_dir_all(&base).map_err(|e|e.to_string())?;
    let dir=base.join("messaging-w4-pilot-v1");
    match fs::symlink_metadata(&dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {},
        Ok(_) => return Err("подмена каталога W4".into()),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => {
            #[cfg(unix)] {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e|e.to_string())?;
            }
            #[cfg(not(unix))] {
                return Err("W4 пока требует Unix-хранилище секретных ключей".into());
            }
        },
        Err(e) => return Err(e.to_string()),
    }
    let meta=fs::symlink_metadata(&dir).map_err(|e|e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink(){return Err("подмена каталога".into());}
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode()&0o077!=0 {
            return Err("закройте каталог W4 для других пользователей: chmod 700".into());
        }
    }
    Ok(dir)
}
fn w4_args(a:&Action,root:&Path)->Vec<String>{
    let sender=root.join("sender.keys").to_string_lossy().into_owned();
    let recipient=root.join("recipient.keys").to_string_lossy().into_owned();
    let out=root.join("sender.sqlite").to_string_lossy().into_owned();
    let inbox=root.join("recipient.sqlite").to_string_lossy().into_owned();
    match a {
        Action::Status=>vec![],
        Action::InitSender=>vec!["init-sender".into(),sender],
        Action::InitRecipient=>vec!["init-recipient".into(),recipient],
        Action::Listen{key,drop_ack}=>vec![
            (if *drop_ack { "serve-drop-ack" } else { "serve-once" }).into(),
            recipient,inbox,key.clone(),"127.0.0.1:42042".into(),
        ],
        Action::Send{invite}=>vec![
            "send-new".into(),sender,out,invite.clone(),"127.0.0.1:42043".into(),"0".repeat(1024),
        ],
        Action::Resend{invite,message}=>vec![
            "resend".into(),sender,out,invite.clone(),"127.0.0.1:42043".into(),message.clone(),
        ],
    }
}
/// Only called from an explicit --w4-pilot invocation, before GUI startup.
pub fn run()->i32{
    if env::var_os("AKULA_DISABLE_W4_PILOT").as_deref()==Some(OsStr::new("1")) {
        eprintln!("W4 отключён переменной AKULA_DISABLE_W4_PILOT=1");
        return 2;
    }
    let a:Vec<String>=match env::args_os().skip(2).map(|a|a.into_string()).collect(){
        Ok(v)=>v,
        Err(_)=>{eprintln!("недопустимый параметр W4");return 2;},
    };
    let action=match parse(&a){
        Ok(v)=>v,Err(e)=>{eprintln!("{e}");return 2;},
    };
    let executable=match helper(){
        Ok(v)=>v,Err(e)=>{eprintln!("Akula W4: {e}");return 2;},
    };
    if action==Action::Status {
        println!("AKULA_W4_PILOT_READY helper=verified network=inactive auto_start=no SSH_only=yes");
        return 0;
    }
    let root=match private_root(){
        Ok(v)=>v,Err(e)=>{eprintln!("Akula W4: {e}");return 2;},
    };
    match Command::new(executable).args(w4_args(&action,&root))
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status() {
        Ok(s)=>s.code().unwrap_or(1),
        Err(e)=>{eprintln!("Не удалось запустить W4: {e}");1}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_network_command_or_freeform_host_is_accepted(){
        assert_eq!(parse(&["status".into()]),Ok(Action::Status));
        assert!(parse(&["send-test".into(),"a".repeat(160)]).is_ok());
        for invalid in [
            vec!["send-test".into(),"a".repeat(159)],
            vec!["send-test".into(),"a".repeat(160),"evil".into()],
            vec!["listen".into(),"0.0.0.0:8080".into()],
            vec!["listen".into(),"; rm -rf ~".into()],
            vec!["revoke".into()],
        ] { assert!(parse(&invalid).is_err()); }
        for action in [
            Action::Listen{key:"a".repeat(64),drop_ack:false},
            Action::Send{invite:"b".repeat(160)},
            Action::Resend{invite:"b".repeat(160),message:"c".repeat(32)},
        ] {
            let args=w4_args(&action,Path::new("/tmp/akula-private"));
            assert!(args.iter().any(|s|s=="127.0.0.1:42042"||s=="127.0.0.1:42043"));
            assert!(args.iter().all(|s|!s.contains("0.0.0.0")));
        }
    }
    #[test]
    fn unauthenticated_or_replaced_binary_fails_closed(){
        let temp=tempfile::tempdir().unwrap();
        let helper=temp.path().join("akula-w4-lab");
        assert!(verify_executable(&helper,Some(&"0".repeat(64))).is_err());
        fs::write(&helper,b"not the verified WOW helper").unwrap();
        assert!(verify_executable(&helper,None).is_err());
        assert!(verify_executable(&helper,Some(&"0".repeat(64))).is_err());
        assert!(verify_executable(&helper,Some(&Sha256::digest(b"not the verified WOW helper").iter().map(|byte| format!("{byte:02x}")).collect::<String>())).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_helper_is_refused(){
        use std::os::unix::fs::symlink;
        let temp=tempfile::tempdir().unwrap();
        let target=temp.path().join("real");
        fs::write(&target,b"binary").unwrap();
        let link=temp.path().join("akula-w4-lab");
        symlink(&target,&link).unwrap();
        assert!(verify_executable(&link,Some(&"a".repeat(64))).is_err());
    }
}
