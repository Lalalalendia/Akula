#[cfg(target_os = "linux")]
mod linux;
mod memory;
#[cfg(windows)]
mod windows;

use super::{Error, Result};
use serde::{Deserialize, Serialize};
use std::process::{Child, Command};

const MIB: u64 = 1024 * 1024;
pub(super) const MAX_THREADS: u8 = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuMode {
    Quiet,
    #[default]
    Balanced,
    Fast,
}

impl CpuMode {
    pub fn threads(self) -> u8 {
        self.for_cpus(std::thread::available_parallelism().map_or(1, |count| count.get()))
    }

    fn for_cpus(self, cpus: usize) -> u8 {
        let threads = match self {
            Self::Quiet => 1,
            Self::Balanced => cpus / 2,
            Self::Fast => cpus
                .saturating_sub(2)
                .min(cpus.saturating_sub(cpus.div_ceil(4))),
        };
        threads.clamp(1, usize::from(MAX_THREADS)) as u8
    }
}

#[derive(Clone, Copy)]
pub(super) enum Limits {
    Media,
    Recognition(u8),
}

impl Limits {
    pub(super) fn memory(self) -> Result<usize> {
        let (floor, ceiling) = match self {
            Self::Media => (256, 768),
            Self::Recognition(_) => (512, 4096),
        };
        let bytes = memory::budget(memory::available(), floor, ceiling)?;
        Ok(bytes.min(usize::MAX as u64) as usize)
    }

    pub(super) fn spawn(self, command: &mut Command) -> Result<Running> {
        let memory = self.memory()?;
        let threads = match self {
            Self::Media => 1,
            Self::Recognition(threads) => threads,
        };
        let count = threads.to_string();
        for key in [
            "OMP_NUM_THREADS",
            "OMP_THREAD_LIMIT",
            "OPENBLAS_NUM_THREADS",
            "MKL_NUM_THREADS",
            "BLIS_NUM_THREADS",
            "VECLIB_MAXIMUM_THREADS",
        ] {
            command.env(key, &count);
        }
        command
            .env("OMP_DYNAMIC", "FALSE")
            .env("OMP_WAIT_POLICY", "PASSIVE");
        #[cfg(windows)]
        {
            let (child, job) = windows::spawn(command, memory, threads)?;
            Ok(Running { child, _job: job })
        }
        #[cfg(not(windows))]
        {
            #[cfg(target_os = "linux")]
            linux::limit(command, memory, threads)?;
            #[cfg(not(target_os = "linux"))]
            let _ = memory;
            Ok(Running {
                child: command.spawn()?,
            })
        }
    }
}

pub(super) struct Running {
    pub(super) child: Child,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests;

impl crate::background_jobs::Queue {
    pub(super) fn park_for_memory(
        &self,
        lease: &crate::background_jobs::Lease,
    ) -> crate::background_jobs::Result<()> {
        use crate::background_jobs::changed;
        self.timed_write(|conn, now| {
            changed(conn.execute(
                "UPDATE jobs SET state='paused', lease_until=NULL
             WHERE id=?1 AND generation=?2 AND state='running' AND lease_until>?3",
                rusqlite::params![lease.job, lease.generation, now],
            )?)
        })
    }
}
