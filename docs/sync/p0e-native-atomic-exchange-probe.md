SYNC-APPLY-P0e native swap lab (test only)

This module only compiles under cfg(test). It cannot be called from the production editor, S3 transport, File Gate or VaultPort. It exchanges file names ONLY in temporary test fixtures using a pinned parent descriptor. Linux uses renameat2 RENAME_EXCHANGE; macOS uses renameatx_np RENAME_SWAP. Windows fails closed and does not use ReplaceFileW.

A real swap is atomic as a name operation, but is NOT compare-and-swap on the previous file identity. The tests preserve the old inode and exact old bytes at a unique stage name, verify both new and old contents after swap, and fsync the parent. A deliberate panic after exchange proves that a lost acknowledgement must be a durable HOLD, never an automatic retry or rollback. Unsupported filesystems must reject rather than fall back to plain rename. No receipt or ConfirmedBaseline is issued.

Production requires registered host/root custody through the entire write, File Gate reservation, P0d immutable attempt claim, protected original out of the vault, pinned identity with OS-specific native evidence, and independent post-write readback. Same-user hostile namespace races remain outside the cooperative model.

References: https://man7.org/linux/man-pages/man2/renameat2.2.html and https://newosxbook.com/src.php?file=%2Fbsd%2Fman%2Fman2%2Frename.2&tree=xnu and https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew
