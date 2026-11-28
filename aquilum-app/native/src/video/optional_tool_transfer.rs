//! Bounded, SHA-pinned HTTP transfer to a private staging file; no publication.
use sha2::{Digest, Sha256};
use std::{fs::File, io::{Read, Write}, time::Duration};

pub(crate) struct Transfer<'a> {
    pub url: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
    pub notice_every: u64,
    pub errors: [&'a str; 7],
}

impl Transfer<'_> {
    pub(crate) fn download(&self, dest: File, notice: impl FnMut(u64)) -> Result<(), String> {
        // Redirects are allowed, but exact length and hash must verify before publication.
        let [request, status, ..] = self.errors;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(30))
            .build();
        let reply = agent.get(self.url).call().map_err(|e| format!("{request}: {e}"))?;
        if reply.status() != 200 {
            return Err(status.into());
        }
        self.copy_checked(&mut reply.into_reader(), dest, notice)
    }

    pub(crate) fn copy_checked(&self, stream: &mut impl Read, mut dest: File,
        mut notice: impl FnMut(u64)) -> Result<(), String> {
        let [_, _, interrupted, oversize, write, flush, mismatch] = self.errors;
        let mut digest = Sha256::new();
        let mut done = 0u64;
        let mut last_notice = 0u64;
        let mut buffer = [0u8; 65_536];
        loop {
            let n = stream.read(&mut buffer).map_err(|e| format!("{interrupted}: {e}"))?;
            if n == 0 { break; }
            done = done.saturating_add(n as u64);
            if done > self.bytes { return Err(oversize.into()); }
            digest.update(&buffer[..n]);
            dest.write_all(&buffer[..n]).map_err(|e| format!("{write}: {e}"))?;
            if done.saturating_sub(last_notice) >= self.notice_every {
                last_notice = done;
                notice(done);
            }
        }
        dest.sync_all().map_err(|e| format!("{flush}: {e}"))?;
        drop(dest);
        let hash = digest.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
        if done != self.bytes || hash != self.sha256 {
            return Err(mismatch.into());
        }
        Ok(())
    }
}
