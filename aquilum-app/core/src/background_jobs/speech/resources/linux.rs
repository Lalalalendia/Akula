use rustix::process::{self, CpuSet, Resource, Rlimit, Signal};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

pub(super) fn limit(command: &mut Command, memory: usize, threads: u8) -> io::Result<()> {
    let parent = process::getpid();
    let allowed = process::sched_getaffinity(None)?;
    let mut selected = CpuSet::new();
    for cpu in (0..CpuSet::MAX_CPU)
        .filter(|&cpu| allowed.is_set(cpu))
        .take(usize::from(threads))
    {
        selected.set(cpu);
    }
    // Only syscalls and numeric OS errors are used after fork.
    unsafe {
        command.pre_exec(move || {
            process::set_parent_process_death_signal(Some(Signal::Kill))?;
            if process::getppid() != Some(parent) {
                return Err(rustix::io::Errno::SRCH.into());
            }
            let inherited = process::getrlimit(Resource::As);
            let memory = memory as u64;
            let capped = |value: Option<u64>| Some(value.unwrap_or(memory).min(memory));
            process::setrlimit(
                Resource::As,
                Rlimit {
                    current: capped(inherited.current),
                    maximum: capped(inherited.maximum),
                },
            )?;
            process::sched_setaffinity(None, &selected)?;
            process::nice(10)?;
            Ok(())
        });
    }
    Ok(())
}
