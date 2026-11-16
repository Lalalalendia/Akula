//! VIDEO-06 non-release: combine separate fragmented MP4/DASH AVC and AAC
//! tracks using only Rust (no FFmpeg in remux path). The test generator can
//! use FFmpeg as a source of *synthetic fixtures*, not as a product dependency.
use mp4::{AacConfig, AvcConfig, MediaType, Mp4Config, Mp4Reader, Mp4Sample, Mp4Track, Mp4Writer, TrackConfig};
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Reader = Mp4Reader<BufReader<File>>;

fn open(path: &Path) -> Result<Reader> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    Ok(Mp4Reader::read_header(BufReader::new(file), length)?)
}

fn box_fragments(path: &Path) -> Result<usize> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut offset = 0u64;
    let mut moofs = 0usize;
    while offset < size {
        if size - offset < 8 { return Err("truncated MP4 top-level header".into()); }
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let short = u32::from_be_bytes(header[..4].try_into()?);
        let header_len = if short == 1 { 16u64 } else { 8u64 };
        let length = if short == 1 {
            let mut extended = [0u8; 8];
            file.read_exact(&mut extended)?;
            u64::from_be_bytes(extended)
        } else if short == 0 { size - offset } else { u64::from(short) };
        if length < header_len || length > size - offset { return Err("invalid top-level MP4 box size".into()); }
        if &header[4..8] == b"moof" { moofs += 1; }
        offset = offset.checked_add(length).ok_or("MP4 box offset overflow")?;
    }
    if moofs < 2 { return Err("expected multiple real fragmented MP4 moof boxes".into()); }
    Ok(moofs)
}

fn config(track: &Mp4Track, codec: MediaType) -> Result<TrackConfig> {
    let mut cfg = match codec {
        MediaType::H264 => TrackConfig::from(AvcConfig {
            width: track.width(),
            height: track.height(),
            seq_param_set: track.sequence_parameter_set()?.to_vec(),
            pic_param_set: track.picture_parameter_set()?.to_vec(),
        }),
        MediaType::AAC => TrackConfig::from(AacConfig {
            bitrate: track.bitrate(),
            profile: track.audio_profile()?,
            freq_index: track.sample_freq_index()?,
            chan_conf: track.channel_config()?,
        }),
        _ => return Err("only H264/AAC are accepted".into()),
    };
    cfg.timescale = track.timescale();
    cfg.language = track.language().to_owned();
    if cfg.timescale == 0 { return Err("invalid track timescale".into()); }
    Ok(cfg)
}

struct Stream {
    reader: Reader,
    id: u32,
    total: u32,
    next_idx: u32,
    cfg: TrackConfig,
}
impl Stream {
    fn new(path: &Path, codec: MediaType) -> Result<Self> {
        let reader = open(path)?;
        let found = reader.tracks().values().find_map(|t|
            (t.media_type().ok() == Some(codec)).then(|| (t.track_id(), t))
        ).ok_or("required media track missing")?;
        let (id, t) = found;
        let cfg = config(t, codec)?;
        let total = t.sample_count();
        if total == 0 { return Err("empty track not allowed".into()); }
        Ok(Self { reader, id, total, next_idx: 1, cfg })
    }
    fn next(&mut self) -> Result<Option<Mp4Sample>> {
        if self.next_idx > self.total { return Ok(None); }
        let index = self.next_idx;
        let sample = self.reader.read_sample(self.id, index)?
            .ok_or("missing fragmented sample")?;
        self.next_idx += 1;
        Ok(Some(sample))
    }
}

fn make_writer(path: &Path) -> Result<Mp4Writer<File>> {
    let cfg = Mp4Config {
        major_brand: "isom".parse()?,
        minor_version: 512,
        compatible_brands: ["isom", "iso2", "avc1", "mp41"]
            .iter().map(|x| x.parse()).collect::<mp4::Result<Vec<_>>>()?,
        timescale: 1000,
    };
    Ok(Mp4Writer::write_start(File::create(path)?, &cfg)?)
}

fn merge(video_path: &Path, audio_path: &Path, output: &Path) -> Result<()> {
    let mut v = Stream::new(video_path, MediaType::H264)?;
    let mut a = Stream::new(audio_path, MediaType::AAC)?;
    let vcount = v.total;
    let acount = a.total;
    let (vscale, ascale) = (v.cfg.timescale, a.cfg.timescale);
    let mut w = make_writer(output)?;
    w.add_track(&v.cfg)?;
    w.add_track(&a.cfg)?;
    let (mut vs, mut aus) = (v.next()?, a.next()?);
    let mut written = (0, 0);
    while vs.is_some() || aus.is_some() {
        let choose_video = match (&vs, &aus) {
            (Some(x), Some(y)) =>
                u128::from(x.start_time) * u128::from(ascale)
                    <= u128::from(y.start_time) * u128::from(vscale),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if choose_video {
            w.write_sample(1, vs.as_ref().ok_or("missing video")?)?;
            written.0 += 1;
            vs = v.next()?;
        } else {
            w.write_sample(2, aus.as_ref().ok_or("missing audio")?)?;
            written.1 += 1;
            aus = a.next()?;
        }
    }
    w.write_end()?;
    if written != (vcount, acount) { return Err("not all source samples were written".into()); }
    println!("DASH_RUST_MUXED video_samples={} audio_samples={} output_bytes={}",
             vcount, acount, output.metadata()?.len());
    Ok(())
}

fn verify_stream(input: &Path, output: &Path, codec: MediaType) -> Result<()> {
    let mut original = Stream::new(input, codec)?;
    let mut result = Stream::new(output, codec)?;
    if original.total != result.total { return Err("sample count differs after remux".into()); }
    let count = original.total;
    for _ in 0..count {
        let a = original.next()?.ok_or("original sample missing")?;
        let b = result.next()?.ok_or("remuxed sample missing")?;
        if a.bytes != b.bytes || a.start_time != b.start_time
            || a.duration != b.duration || a.rendering_offset != b.rendering_offset
            || a.is_sync != b.is_sync
        {
            return Err(format!("sample content or timing changed for {codec:?}").into());
        }
    }
    println!("DASH_SAMPLE_FIDELITY_PASS codec={codec:?} samples={count}");
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 || args[1] != "prove-pair" {
        return Err("usage: dash prove-pair <fragmented-video.mp4> <fragmented-audio.m4a> <output.mp4>".into());
    }
    let video = Path::new(&args[2]);
    let audio = Path::new(&args[3]);
    let result = Path::new(&args[4]);
    let vf = box_fragments(video)?;
    let af = box_fragments(audio)?;
    merge(video, audio, result)?;
    verify_stream(video, result, MediaType::H264)?;
    verify_stream(audio, result, MediaType::AAC)?;
    let readback = open(result)?;
    if readback.tracks().len() != 2 { return Err("expected exactly two output tracks".into()); }
    println!("DASH_RUST_FRAGMENTED_REMUX_PASS video_fragments={vf} audio_fragments={af} ffmpeg_in_runtime=0");
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("DASH_RUST_FAILURE: {err}");
        std::process::exit(1);
    }
}
