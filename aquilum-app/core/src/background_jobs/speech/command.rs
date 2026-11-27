use super::{Error, Result};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(super) struct Workspace(PathBuf);

impl Workspace {
    pub(super) fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("akula-asr-{}", uuid::Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        builder.recursive(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        Ok(Self(path))
    }

    pub(super) fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub(super) fn run(
    command: &mut Command,
    workspace: &Workspace,
    stdout: Option<&Path>,
    output: (&Path, u64),
    timeout: Duration,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<()> {
    const LOG_LIMIT: u64 = 4 * 1024 * 1024;
    let stderr = workspace.file("stderr.log");
    let out = match stdout {
        Some(path) => Stdio::from(File::create(path)?),
        None => Stdio::null(),
    };
    let label = command.get_program().to_string_lossy().into_owned();
    check()?;
    #[cfg(target_os = "linux")]
    limit_process(command);
    let mut child = Running(
        command
            .current_dir(&workspace.0)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(File::create(&stderr)?)
            .spawn()?,
    );
    let start = Instant::now();
    loop {
        if start.elapsed() >= timeout {
            return Err(Error::Invalid(format!("{label} exceeded its time limit")));
        }
        check()?;
        if length(output.0)? > output.1 || length(&stderr)? > LOG_LIMIT {
            return Err(Error::Invalid(format!("{label} exceeded its output limit")));
        }
        if let Some(status) = child.0.try_wait()? {
            if status.success() {
                if length(output.0)? > output.1 {
                    return Err(Error::Invalid(format!("{label} exceeded its output limit")));
                }
                return Ok(());
            }
            use std::io::Read;
            let mut diagnostic = Vec::new();
            File::open(stderr)?
                .take(1024)
                .read_to_end(&mut diagnostic)?;
            return Err(Error::Invalid(format!(
                "{label} exited with {status}: {}",
                String::from_utf8_lossy(&diagnostic).trim()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "linux")]
fn limit_process(command: &mut Command) {
    use rustix::process::{self, Resource, Rlimit, Signal};
    use std::os::unix::process::CommandExt;

    const ADDRESS_SPACE: u64 = 4 * 1024 * 1024 * 1024;
    let parent = process::getpid();
    // SAFETY: the child performs only Linux syscalls and creates errors from
    // numeric OS codes between fork and exec.
    unsafe {
        command.pre_exec(move || {
            process::set_parent_process_death_signal(Some(Signal::Kill))?;
            if process::getppid() != Some(parent) {
                return Err(rustix::io::Errno::SRCH.into());
            }
            let inherited = process::getrlimit(Resource::As);
            let limit = |value: Option<u64>| value.unwrap_or(ADDRESS_SPACE).min(ADDRESS_SPACE);
            process::setrlimit(
                Resource::As,
                Rlimit {
                    current: Some(limit(inherited.current)),
                    maximum: Some(limit(inherited.maximum)),
                },
            )?;
            process::nice(10)?;
            Ok(())
        });
    }
}

fn length(path: &Path) -> Result<u64> {
    match fs::metadata(path) {
        Ok(info) => Ok(info.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}
