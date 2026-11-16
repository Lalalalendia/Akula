use masonry::peniko::{ImageAlphaType, ImageBrush};
use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFSourceReader, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM,
};
use windows::core::HSTRING;

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

pub struct Frames {
    pub reader: IMFSourceReader,
    width: u32,
    height: u32,
    stride: i32,
    next: Option<(f64, IMFSample)>,
    ended: bool,
}

impl Frames {
    pub fn open(source: &HSTRING) -> Option<Frames> {
        let (reader, width, height, stride) = super::frames_reader::open(source)?;
        Some(Frames { reader, width, height, stride, next: None, ended: false })
    }

    pub fn ended(&self) -> bool {
        self.ended && self.next.is_none()
    }

    pub fn seek(&mut self, seconds: f64) {
        self.next = None;
        self.ended = false;
        super::worker::seek(&self.reader, seconds);
    }

    fn read(&mut self) -> Option<(f64, IMFSample)> {
        unsafe {
            loop {
                let (mut flags, mut time) = (0u32, 0i64);
                let mut sample = None;
                if self.reader.ReadSample(STREAM, 0, None, Some(&mut flags), Some(&mut time), Some(&mut sample)).is_err() || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    self.ended = true;
                    return None;
                }
                if let Some(sample) = sample {
                    return Some((time as f64 / 1e7, sample));
                }
            }
        }
    }

    pub fn until(&mut self, clock: f64) -> Option<ImageBrush> {
        let mut shown = None;
        loop {
            if self.next.is_none() && !self.ended {
                self.next = self.read();
            }
            match &self.next {
                Some((time, _)) if *time <= clock => shown = self.next.take().map(|(_, sample)| sample),
                _ => break,
            }
        }
        shown.and_then(|sample| self.image(&sample))
    }

    fn image(&self, sample: &IMFSample) -> Option<ImageBrush> {
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer().ok()?;
            let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut data, None, Some(&mut length)).ok()?;
            let bytes = std::slice::from_raw_parts(data, length as usize);
            let (row, pitch, height) = ((self.width * 4) as usize, self.stride.unsigned_abs() as usize, self.height as usize);
            let mut pixels = vec![0u8; row * height];
            for (y, line) in pixels.chunks_exact_mut(row).enumerate() {
                let start = if self.stride < 0 { height - 1 - y } else { y } * pitch;
                if let Some(source) = bytes.get(start..start + row) {
                    line.copy_from_slice(source);
                }
            }
            let _ = buffer.Unlock();
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            crate::render::from_rgba(pixels, self.width, self.height, ImageAlphaType::Alpha)
        }
    }
}
