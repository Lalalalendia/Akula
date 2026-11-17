//! Configure Windows Media Foundation video decoding with a Windows 7 fallback.
//! Advanced processing (Windows 8+) can scale frames. The Windows 7 path only
//! converts YUV to RGB32, so it must never request a resized output format.
use windows::Win32::Media::MediaFoundation::{
    IMFSourceReader, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromURL,
    MFMediaType_Video, MFVideoFormat_RGB32,
};
use windows::core::HSTRING;

const MAX_WIDTH: u32 = 1280;
const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
type Opened = (IMFSourceReader, u32, u32, i32);

pub(super) fn open(source: &HSTRING) -> Option<Opened> {
    open_mode(source, true).or_else(|| open_mode(source, false))
}

fn open_mode(source: &HSTRING, advanced: bool) -> Option<Opened> {
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1).ok()?;
        let attributes = attributes?;
        let setting = if advanced {
            &MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING
        } else {
            &MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING
        };
        attributes.SetUINT32(setting, 1).ok()?;
        let reader = MFCreateSourceReaderFromURL(source, &attributes).ok()?;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).ok()?;
        reader.SetStreamSelection(STREAM, true).ok()?;
        let native = reader.GetNativeMediaType(STREAM, 0).ok()?;
        let size = native.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let (w, h) = ((size >> 32) as u32, size as u32);
        if w == 0 || h == 0 || (!advanced && w > MAX_WIDTH) { return None; }
        let (out_w, out_h) = if advanced && w > MAX_WIDTH {
            (MAX_WIDTH, ((u64::from(h) * u64::from(MAX_WIDTH) / u64::from(w)) as u32).max(2) & !1)
        } else {
            (w, h)
        };
        let kind = MFCreateMediaType().ok()?;
        kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).ok()?;
        kind.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).ok()?;
        kind.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(out_w) << 32) | u64::from(out_h)).ok()?;
        reader.SetCurrentMediaType(STREAM, None, &kind).ok()?;
        let current = reader.GetCurrentMediaType(STREAM).ok()?;
        let frame_size = current.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let (width, height) = ((frame_size >> 32) as u32, frame_size as u32);
        if width == 0 || height == 0 || width > MAX_WIDTH { return None; }
        let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or((width * 4) as i32, |s| s as i32);
        Some((reader, width, height, stride))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::MediaFoundation::{
        MFStartup, MF_VERSION, MFSTARTUP_FULL, MF_SOURCE_READERF_ENDOFSTREAM,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    #[test]
    fn windows7_basic_rgb32_path_decodes_real_h264_without_resizing() {
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("source.mp4");
        std::fs::write(&video, include_bytes!("../fixtures/native_avc_aac_test.mp4")).unwrap();
        let decoded = std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL).expect("Media Foundation unavailable");
            }
            let video = crate::workspace::normalize(&std::fs::canonicalize(video).unwrap());
            let source = HSTRING::from(video.as_os_str());
            let (reader, width, height, _) = open_mode(&source, false)
                .expect("Windows 7 basic reader must convert H264 to native RGB32");
            assert!(width > 0 && height > 0 && width <= MAX_WIDTH);
            for _ in 0..60 {
                let mut flags = 0u32;
                let mut sample = None;
                unsafe {
                    reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample))
                        .expect("basic reader could not decode a sample");
                }
                if let Some(sample) = sample {
                    let buf = unsafe { sample.ConvertToContiguousBuffer() }
                        .expect("basic reader did not return RGB32 data");
                    let count = unsafe { buf.GetCurrentLength() }.unwrap();
                    return usize::try_from(count).unwrap() >= width as usize * height as usize * 4;
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 { break; }
            }
            false
        }).join().expect("basic decoder test thread panicked");
        assert!(decoded, "Windows 7 basic fallback produced no RGB32 frame");
    }
}
