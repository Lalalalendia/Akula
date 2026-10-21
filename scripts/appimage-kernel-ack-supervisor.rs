// RELEASE-LINUX-ACK-PASSCRED-01 — CI-only real AppImage GUI receiver.
// Never use this as the production installer or to publish new files.
// The expected package hash is a CI-computed witness, not publisher proof.

#![cfg(target_os = "linux")]

#[allow(dead_code)]
#[path = "../aquilum-app/native/src/updater/linux_ack_receiver.rs"]
mod ack_receiver;

use ack_receiver::AckReceiver;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

const CHILD_FD: i32 = 198;
unsafe extern "C" {
    fn dup2(old: i32, new: i32) -> i32;
    fn setsid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

fn hold(message: &str) -> String {
    format!("AKULA_LINUX_KERNEL_GUI_E2E_HOLD: {message}")
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() != 5
        || std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("AKULA_DISPOSABLE_ACK_TEST").as_deref() != Ok("1")
    {
        return Err(hold("not an isolated GitHub-hosted four-argument GUI acceptance run"));
    }
    let app = Path::new(&arguments[1]);
    let image_name = app.file_name().and_then(|x| x.to_str())
        .ok_or_else(|| hold("bad package filename"))?;
    if !app.is_absolute()
        || !image_name.starts_with("akula-app_")
        || !image_name.ends_with("_x86_64.AppImage")
    {
        return Err(hold("must run the real candidate AppImage from an absolute path"));
    }
    let info = std::fs::symlink_metadata(app).map_err(|e| e.to_string())?;
    if !info.is_file() || info.file_type().is_symlink() || info.len() == 0 {
        return Err(hold("candidate must be a nonempty regular AppImage"));
    }
    let digest = arguments[2].to_str().ok_or_else(|| hold("non-UTF8 digest"))?;
    let vault = Path::new(&arguments[3]);
    let document = Path::new(&arguments[4]);
    if vault != Path::new("/tmp/vault")
        || document != Path::new("/tmp/vault/First.md")
        || !document.is_file()
    {
        return Err(hold("only synthetic CI Markdown fixture is permitted"));
    }

    let (mut verifier, socket_for_child) = AckReceiver::new(digest)?;
    let source = socket_for_child.as_raw_fd();
    let output = std::fs::File::create("/tmp/akula-native-passcred-child.log")
        .map_err(|e| e.to_string())?;
    let error_output = output.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(app);
    command.arg("--vault").arg(vault).arg("--open").arg(document)
        .envs(verifier.challenge_environment(CHILD_FD)?)
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .stdout(Stdio::from(output))
        .stderr(Stdio::from(error_output))
        .stdin(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if dup2(source, CHILD_FD) < 0 || setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut launched = command.spawn().map_err(|e| e.to_string())?;
    drop(socket_for_child);
    let result = (|| {
        let evidence = verifier.receive_once(&mut launched)?;
        if evidence.candidate_digest != digest || evidence.actual_gui_pid <= 1 {
            return Err(hold("ACK witness differs from expected AppImage generation"));
        }
        println!("AKULA_LINUX_KERNEL_REAL_GUI_ACK_RECEIVED");
        io::stdout().flush().map_err(|e| e.to_string())?;
        let status = launched.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(hold("real AppImage GUI returned nonzero after close"));
        }
        println!("AKULA_LINUX_KERNEL_REAL_GUI_ACK_PASS passcred=1 pidfd=1 os_tree=1 nonce=1 digest=1");
        Ok(())
    })();
    if result.is_err() {
        // Killing just the wrapper may leave an AppImage-extracted GUI behind.
        // This process was launched as a new session leader.
        unsafe { kill(-(launched.id() as i32), 9); }
        let _ = launched.wait();
    }
    result
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
