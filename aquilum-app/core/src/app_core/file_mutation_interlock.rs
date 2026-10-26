use super::Core;
use crate::files::error::FileCommandError;
use crate::files::gate::FileGateInterprocessLock;
use std::sync::{Mutex, MutexGuard};
#[cfg(test)]
use std::sync::TryLockError;

/// Local mutex plus a stable OS file lock shared by independent Akula
/// processes. The OS lock coordinates writers; it does not authenticate or
/// authorize them.
pub(crate) struct FileMutationInterlock {
    process: Mutex<()>,
}

impl FileMutationInterlock {
    pub(crate) const fn new() -> Self {
        Self {
            process: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> Result<FileMutationGuard<'_>, FileMutationLockError> {
        let process = self
            .process
            .lock()
            .map_err(|_| FileMutationLockError::Poisoned)?;
        let interprocess =
            FileGateInterprocessLock::acquire().map_err(FileMutationLockError::Interprocess)?;
        Ok(FileMutationGuard {
            process: Some(process),
            _interprocess: interprocess,
        })
    }

    #[cfg(test)]
    pub(crate) fn try_lock(
        &self,
    ) -> Result<FileMutationGuard<'_>, TryLockError<MutexGuard<'_, ()>>> {
        let process = self.process.try_lock()?;
        let interprocess = match FileGateInterprocessLock::try_acquire() {
            Ok(Some(interprocess)) => interprocess,
            Ok(None) => {
                drop(process);
                return Err(TryLockError::WouldBlock);
            }
            Err(error) => panic!("interprocess File Gate lock probe failed: {error}"),
        };
        Ok(FileMutationGuard {
            process: Some(process),
            _interprocess: interprocess,
        })
    }

    #[cfg(test)]
    pub(crate) fn try_process_only_lock(
        &self,
    ) -> Result<MutexGuard<'_, ()>, TryLockError<MutexGuard<'_, ()>>> {
        self.process.try_lock()
    }
}

#[derive(Debug)]
pub(crate) enum FileMutationLockError {
    Poisoned,
    Interprocess(std::io::Error),
}

impl std::fmt::Display for FileMutationLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Poisoned => formatter.write_str("process-local File Gate mutex is poisoned"),
            Self::Interprocess(error) => {
                write!(formatter, "interprocess File Gate lock failed: {error}")
            }
        }
    }
}

pub(crate) struct FileMutationGuard<'a> {
    process: Option<MutexGuard<'a, ()>>,
    _interprocess: FileGateInterprocessLock,
}

impl FileMutationGuard<'_> {
    /// Keep the cross-process File Gate epoch through history publication
    /// while releasing the shorter process-local mutex before callbacks.
    pub(crate) fn release_process_lock(&mut self) {
        self.process.take();
    }
}

impl Core {
    /// Acquire the shared File Gate interlock. The process-local mutex is for
    /// short disk sections; the OS lock may remain held through history.
    pub(crate) fn file_mutation_guard(&self) -> Result<FileMutationGuard<'_>, FileCommandError> {
        self.file_mutations
            .lock()
            .map_err(|error| FileCommandError::Task {
                message: format!(
                    "Core File Gate interlock unavailable; refusing mutation: {error}"
                ),
            })
    }
}
