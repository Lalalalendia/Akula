//! VIDEO-06 non-release: bounded pure-Rust remux of separate H264/AAC
//! fragmented DASH MP4 tracks. FFmpeg is used by CI only to create fixtures.
use mp4_track::{
    Codec, Edit, Limits, Mp4Reader, Mp4Writer, Sample, SampleInput, Track, TrackConfig,
    TrackId, TrackKind, WriterConfig,
};
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Reader = Mp4Reader<BufReader<File>>;

fn constrained_reader(path: &Path) -> Result<Reader> {
    // Limits is non-exhaustive: start with upstream defaults and tighten them.
    let mut limit = Limits::default();
    limit.max_moov_bytes = 8 * 1024 * 1024;
    limit.max_moof_bytes = 2 * 1024 * 1024;
    limit.max_sample_bytes = 8 * 1024 * 1024;
    limit.max_fragments = 20_000;
    limit.max_sample_count = 400_000;
    limit.max_depth = 20;
    Ok(Mp4Reader::open_with(BufReader::new(File::open(path)?), limit)?)
}

fn accepts(track: &Track, kind: TrackKind) -> bool {
    match (track.kind, &track.codec, kind) {
        (TrackKind::Video, Codec::H264 { .. }, TrackKind::Video) => true,
        (TrackKind::Audio, Codec::Aac { .. }, TrackKind::Audio) => true,
        _ => false,
    }
}

const OUTPUT_MOVIE_TIMESCALE: u32 = 1000;

// DASH initialization segments may contain a one-entry edit list with
// segment_duration=0 even though the later moof/trun samples have a real
// duration. Reusing that edit verbatim made mvhd/tkhd duration zero and
// rendered the output unplayable in FFmpeg and common native players.
// Preserve the media start (e.g. AAC's 1024-sample encoder priming offset),
// but derive a valid movie duration from the actual samples. Complex,
// multi-edit timelines are rejected rather than silently desynchronized.
fn normalize_edits(
    edits: &[Edit], source_movie_scale: u32, media_scale: u32, media_duration: u64,
) -> Result<Vec<Edit>> {
    if edits.is_empty() { return Ok(Vec::new()); }
    if edits.len() != 1 || source_movie_scale == 0 || media_scale == 0 {
        return Err("complex or invalid MP4 edit list is not supported".into());
    }
    let mut edit = edits[0].clone();
    if edit.media_rate != 1.0 {
        return Err("non-unity MP4 edit playback rate is unsupported".into());
    }
    let start = edit.media_time.ok_or("empty edit timelines are not supported")?;
    let available = media_duration.checked_sub(start)
        .filter(|v| *v > 0).ok_or("MP4 edit media start exceeds track duration")?;
    let media_ms = (u128::from(available) * u128::from(OUTPUT_MOVIE_TIMESCALE))
        / u128::from(media_scale);
    // Positive edits are expressed in the input movie timebase, while
    // the new MP4 writer always uses 1000 movie units per second.
    let requested_ms = if edit.segment_duration == 0 {
        media_ms
    } else {
        (u128::from(edit.segment_duration) * u128::from(OUTPUT_MOVIE_TIMESCALE))
            / u128::from(source_movie_scale)
    };
    if requested_ms == 0 || requested_ms > media_ms {
        return Err("MP4 edit would create a zero-duration or overlong output".into());
    }
    edit.segment_duration = u64::try_from(requested_ms)?;
    Ok(vec![edit])
}

fn configure(
    track: &Track, source_movie_scale: u32, media_duration: u64,
) -> Result<TrackConfig> {
    Ok(TrackConfig {
        kind: track.kind,
        timescale: track.timescale,
        language: track.language.clone(),
        handler_name: track.handler_name.clone(),
        codec: track.codec.clone(),
        width: track.width,
        height: track.height,
        edit_list: normalize_edits(
            &track.edit_list, source_movie_scale, track.timescale, media_duration,
        )?,
    })
}

struct Source {
    reader: Reader,
    id: TrackId,
    count: u32,
    cursor: u32,
    cfg: TrackConfig,
    fragments: usize,
}
impl Source {
    fn new(path: &Path, kind: TrackKind, require_fragments: bool) -> Result<Self> {
        let reader = constrained_reader(path)?;
        if reader.tracks().len() != 1 {
            return Err("separate DASH input must have exactly one track".into());
        }
        let track = reader.tracks().first().ok_or("missing track")?;
        if !accepts(track, kind) || track.timescale == 0 {
            return Err(format!("unsupported DASH track: kind={:?} codec={:?} timescale={}", track.kind, track.codec, track.timescale).into());
        }
        let id = track.id;
        // Fragmented MP4 can have stbl sample_count=0 in moov; actual
        // samples are in moof/trun and exposed by the bounded reader.
        let mut media_duration = 0u64;
        let mut sample_count = 0usize;
        for sample in reader.samples(id)? {
            media_duration = media_duration.checked_add(u64::from(sample.duration))
                .ok_or("MP4 media duration overflow")?;
            sample_count += 1;
        }
        let count = u32::try_from(sample_count)?;
        if count == 0 || media_duration == 0 {
            return Err("DASH track has no duration-bearing samples".into());
        }
        let cfg = configure(track, reader.movie_timescale(), media_duration)?;
        let fragments = reader.fragments().len();
        if require_fragments && fragments < 3 {
            return Err("strict DASH fixture must contain multiple fragments".into());
        }
        if reader.sample_info(id, 0)?.dts != 0 {
            return Err("nonzero initial DTS must be handled before production adoption".into());
        }
        Ok(Self { reader, id, count, cursor: 0, cfg, fragments })
    }

    fn take(&mut self) -> Result<Option<Sample>> {
        if self.cursor >= self.count { return Ok(None); }
        let current = self.reader.read_sample(self.id, self.cursor)?;
        self.cursor = self.cursor.checked_add(1).ok_or("sample cursor overflow")?;
        Ok(Some(current))
    }
}

fn merge(video: &Path, audio: &Path, file: File, strict: bool) -> Result<(u32, u32, usize, usize)> {
    let mut v = Source::new(video, TrackKind::Video, strict)?;
    let mut a = Source::new(audio, TrackKind::Audio, strict)?;
    let (vcount, acount) = (v.count, a.count);
    let (vscale, ascale) = (v.cfg.timescale, a.cfg.timescale);
    let mut writer = Mp4Writer::new(file, WriterConfig::default())?;
    let vid = writer.add_track(v.cfg.clone())?;
    let aid = writer.add_track(a.cfg.clone())?;
    let (mut vs, mut aus) = (v.take()?, a.take()?);
    let mut written = (0u32, 0u32);
    while vs.is_some() || aus.is_some() {
        let choose_video = match (&vs, &aus) {
            (Some(x), Some(y)) =>
                u128::from(x.info.dts) * u128::from(ascale)
                    <= u128::from(y.info.dts) * u128::from(vscale),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if choose_video {
            let sample = vs.as_ref().ok_or("video sample unavailable")?;
            writer.write_sample(vid, &SampleInput {
                data: &sample.data,
                duration: sample.info.duration,
                cts_offset: i32::try_from(sample.info.cts_offset)?,
                is_sync: sample.info.is_sync,
            })?;
            written.0 += 1;
            vs = v.take()?;
        } else {
            let sample = aus.as_ref().ok_or("audio sample unavailable")?;
            writer.write_sample(aid, &SampleInput {
                data: &sample.data,
                duration: sample.info.duration,
                cts_offset: i32::try_from(sample.info.cts_offset)?,
                is_sync: sample.info.is_sync,
            })?;
            written.1 += 1;
            aus = a.take()?;
        }
    }
    writer.finish()?.sync_all()?;
    if written != (vcount, acount) { return Err("not all samples were remuxed".into()); }
    Ok((vcount, acount, v.fragments, a.fragments))
}

fn compare(input: &Path, output: &Path, kind: TrackKind) -> Result<()> {
    let mut before = Source::new(input, kind, true)?;
    let mut after = constrained_reader(output)?;
    let output_track = after.tracks().iter().find(|track| accepts(track, kind))
        .ok_or("remuxed output track missing")?;
    let id = output_track.id;
    let output_count = u32::try_from(after.samples(id)?.len())?;
    if before.count != output_count {
        return Err(format!("sample count changed for {kind:?}: {} vs {output_count}", before.count).into());
    }
    let count = before.count;
    for index in 0..count {
        let old = before.reader.read_sample(before.id, index)?;
        let new = after.read_sample(id, index)?;
        if old.data != new.data || old.info.dts != new.info.dts
            || old.info.duration != new.info.duration
            || old.info.cts_offset != new.info.cts_offset
            || old.info.is_sync != new.info.is_sync
        {
            return Err(format!("compressed bytes or timing changed at {kind:?} sample {index}").into());
        }
    }
    println!("DASH_SAMPLE_FIDELITY_PASS kind={kind:?} samples={count}");
    Ok(())
}

/// A failed remux never leaves a partially-created output with a playable name.
/// create_new also refuses to overwrite a source or another Akula process.
struct OutputCleanup {
    path: PathBuf,
    keep: bool,
}
impl Drop for OutputCleanup {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 || !(args[1] == "prove-pair" || args[1] == "mux-pair") {
        return Err("usage: dash {prove-pair|mux-pair} <H264-only.mp4> <AAC-only.m4a> <new-output.mp4>".into());
    }
    let strict = args[1] == "prove-pair";
    let video = Path::new(&args[2]);
    let audio = Path::new(&args[3]);
    let result = Path::new(&args[4]);
    if !video.is_file() || !audio.is_file() || video == audio {
        return Err("two distinct regular input files are required".into());
    }
    let output_file = OpenOptions::new().write(true).create_new(true).open(result)?;
    let mut cleanup = OutputCleanup { path: result.to_path_buf(), keep: false };
    let (vcount, acount, vfragments, afragments) = merge(video, audio, output_file, strict)?;
    if strict {
        compare(video, result, TrackKind::Video)?;
        compare(audio, result, TrackKind::Audio)?;
    }
    let output = constrained_reader(result)?;
    if output.tracks().len() != 2 {
        return Err("output is not a two-track MP4".into());
    }
    if output.movie_duration() == 0 {
        return Err("Rust MP4 has zero movie duration after DASH remux".into());
    }
    if result.metadata()?.len() < 1024 {
        return Err("output MP4 unexpectedly small".into());
    }
    cleanup.keep = true;
    if strict {
        println!("DASH_RUST_FRAGMENTED_REMUX_PASS video_samples={vcount} audio_samples={acount} video_fragments={vfragments} audio_fragments={afragments} output_bytes={} ffmpeg_in_runtime=0", result.metadata()?.len());
    } else {
        println!("DASH_RUST_MUX_PAIR_PASS video_samples={vcount} audio_samples={acount} video_fragments={vfragments} audio_fragments={afragments} output_bytes={} ffmpeg_in_runtime=0", result.metadata()?.len());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("DASH_RUST_FAILURE: {error}");
        std::process::exit(1);
    }
}


#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn repairs_zero_duration_dash_edit_and_preserves_aac_priming() {
        let source = [Edit {
            segment_duration: 0,
            media_time: Some(1024),
            media_rate: 1.0,
        }];
        let output = normalize_edits(&source, 1000, 16_000, 97_282).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].media_time, Some(1024));
        assert_eq!(output[0].segment_duration, 6016);
        assert_eq!(source[0].segment_duration, 0);
    }

    #[test]
    fn rescale_existing_edit_movie_timebase() {
        let source = [Edit {
            segment_duration: 24_000,
            media_time: Some(0),
            media_rate: 1.0,
        }];
        let output = normalize_edits(&source, 4000, 48_000, 48_000 * 7).unwrap();
        assert_eq!(output[0].segment_duration, 6000);
    }

    #[test]
    fn rejects_empty_complex_or_overlong_edits() {
        let empty = Edit { segment_duration: 0, media_time: None, media_rate: 1.0 };
        assert!(normalize_edits(&[empty], 1000, 16000, 96000).is_err());
        let bad = Edit { segment_duration: 0, media_time: Some(100_000), media_rate: 1.0 };
        assert!(normalize_edits(&[bad], 1000, 16000, 96000).is_err());
        let multi = Edit { segment_duration: 500, media_time: Some(0), media_rate: 1.0 };
        assert!(normalize_edits(&[multi.clone(), multi], 1000, 16000, 96000).is_err());
    }
}
