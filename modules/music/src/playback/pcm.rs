//! Fixed-capacity interleaved PCM16 reference buffer. Single-owner only.
//! Delivered frames are not hardware-rendered receipts.

use super::Ticket;

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BUFFER_MS: u32 = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcmError {
    InvalidFormat,
    InvalidCapacity,
    Allocation,
    InvalidTicket,
    StaleTicket,
    Misaligned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Format {
    rate: u32,
    channels: usize,
}

impl Format {
    pub fn new(rate: u32, channels: usize) -> Result<Self, PcmError> {
        if !(8_000..=384_000).contains(&rate) || !(1..=8).contains(&channels) {
            return Err(PcmError::InvalidFormat);
        }
        Ok(Self { rate, channels })
    }
    pub fn rate(self) -> u32 {
        self.rate
    }
    pub fn channels(self) -> usize {
        self.channels
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pull {
    pub audio_frames: usize,
    pub silence_frames: usize,
}

pub struct PcmBuffer {
    format: Format,
    ticket: Ticket,
    samples: Vec<i16>,
    head: usize,
    len: usize,
}

impl PcmBuffer {
    pub fn new(format: Format, ticket: Ticket, buffer_ms: u32) -> Result<Self, PcmError> {
        if buffer_ms == 0 || buffer_ms > MAX_BUFFER_MS {
            return Err(PcmError::InvalidCapacity);
        }
        if ticket.session() == 0 {
            return Err(PcmError::InvalidTicket);
        }
        let frames = (u64::from(format.rate) * u64::from(buffer_ms)).div_ceil(1_000);
        let samples = usize::try_from(frames)
            .ok()
            .and_then(|n| n.checked_mul(format.channels))
            .filter(|&n| n <= MAX_BYTES / std::mem::size_of::<i16>())
            .ok_or(PcmError::InvalidCapacity)?;
        let mut data = Vec::new();
        data.try_reserve_exact(samples).map_err(|_| PcmError::Allocation)?;
        data.resize(samples, 0);
        Ok(Self {
            format,
            ticket,
            samples: data,
            head: 0,
            len: 0,
        })
    }

    pub fn capacity_frames(&self) -> usize {
        self.samples.len() / self.format.channels
    }
    pub fn queued_frames(&self) -> usize {
        self.len / self.format.channels
    }

    /// Discontinuity (load/seek/stop). Caller must separately quiesce device.
    pub fn reset(&mut self, ticket: Ticket) -> Result<(), PcmError> {
        self.check_new_ticket(ticket)?;
        self.ticket = ticket;
        self.head = 0;
        self.len = 0;
        Ok(())
    }

    /// Same-session pause/resume revision; output authorization is external.
    pub fn retag(&mut self, ticket: Ticket) -> Result<(), PcmError> {
        self.check_new_ticket(ticket)?;
        if ticket.session() == 0 || ticket.session() != self.ticket.session() {
            return Err(PcmError::InvalidTicket);
        }
        self.ticket = ticket;
        Ok(())
    }

    /// Return whole frames accepted; caller retains suffix. Never overwrite.
    pub fn push(&mut self, ticket: Ticket, input: &[i16]) -> Result<usize, PcmError> {
        self.check(ticket, input.len())?;
        let cap = self.samples.len();
        let count = input.len().min(cap - self.len);
        let tail = (self.head + self.len) % cap;
        let first = count.min(cap - tail);
        self.samples[tail..tail + first].copy_from_slice(&input[..first]);
        self.samples[..count - first].copy_from_slice(&input[first..count]);
        self.len += count;
        Ok(count / self.format.channels)
    }

    /// Fill underruns with silence, without allocating or waiting.
    pub fn pull(&mut self, ticket: Ticket, output: &mut [i16]) -> Result<Pull, PcmError> {
        if let Err(error) = self.check(ticket, output.len()) {
            output.fill(0);
            return Err(error);
        }
        let count = output.len().min(self.len);
        let cap = self.samples.len();
        let first = count.min(cap - self.head);
        output[..first].copy_from_slice(&self.samples[self.head..self.head + first]);
        output[first..count].copy_from_slice(&self.samples[..count - first]);
        output[count..].fill(0);
        self.head = (self.head + count) % cap;
        self.len -= count;
        Ok(Pull {
            audio_frames: count / self.format.channels,
            silence_frames: (output.len() - count) / self.format.channels,
        })
    }

    fn check_new_ticket(&self, ticket: Ticket) -> Result<(), PcmError> {
        if ticket.owner() != self.ticket.owner() || ticket.revision() <= self.ticket.revision() {
            return Err(PcmError::StaleTicket);
        }
        Ok(())
    }

    fn check(&self, ticket: Ticket, count: usize) -> Result<(), PcmError> {
        if ticket != self.ticket {
            return Err(PcmError::StaleTicket);
        }
        if ticket.session() == 0 {
            return Err(PcmError::InvalidTicket);
        }
        if count % self.format.channels != 0 {
            return Err(PcmError::Misaligned);
        }
        Ok(())
    }
}
