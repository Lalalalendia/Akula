//! Pin Unix MCP read-only note I/O to held directory descriptors.
use super::Vault;
use std::path::Path;
#[cfg(windows)] mod windows;
mod bounded;
impl Vault {
    /// Only MCP read_note / note_outline use this read-only primitive.
    /// A pinned Unix parent prevents a post-preflight symlink swap from
    /// redirecting a leaf open outside the selected Vault.
    pub(crate) fn read_note_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        #[cfg(unix)]
        {
            read_unix(self, path)
        }
        #[cfg(windows)]
        { windows::read_windows(self, path) }
        #[cfg(not(any(unix, windows)))]
        { Err("MCP pinned Vault read unsupported on this OS".to_owned()) }
    }
}
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::read_unix;
