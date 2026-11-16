//! Isolated FFmpeg-free MP4 split/remux lab, not an Akula product feature.
//! Copies encoded samples only. Does not decode/re-encode, fetch network content,
//! use a workspace directory, or claim compatibility with arbitrary YouTube media.
use mp4::{
    AacConfig, AvcConfig, MediaType, Mp4Config, Mp4Reader, Mp4Sample, Mp4Track,
    Mp4Writer, TrackConfig,
};
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Reader = Mp4Reader<BufReader<File>>;

fn reader(path: &Path) -> Result<Reader> {
    let file = File::open(path)?;
    let bytes = file.metadata()?.len();
    Ok(Mp4Reader::read_header(BufReader::new(file), bytes)?)
}

fn track(reader: &Reader, codec: MediaType) -> Result<(u32, TrackConfig, u32)> {
    for current in reader.tracks().values() {
        if current.media_type()? == codec {
            return Ok((current.track_id(), track_config(current, codec)?, current.sample_count()));
        }
    }
    Err(format!("required codec {codec:?} is absent").into())
}

fn track_config(t: &Mp4Track, codec: MediaType) -> Result<TrackConfig> {
    let mut config = match codec {
        MediaType::H264 => TrackConfig::from(AvcConfig {
            width: t.width(),
            height: t.height(),
            seq_param_set: t.sequence_parameter_set()?.to_vec(),
            pic_param_set: t.picture_parameter_set()?.to_vec(),
        }),
        MediaType::AAC => TrackConfig::from(AacConfig {
            bitrate: t.bitrate(),
            profile: t.audio_profile()?,
            freq_index: t.sample_freq_index()?,
            chan_conf: t.channel_config()?,
        }),
        _ => return Err("only H264 + AAC are supported in this lab".into()),
    };
    config.timescale = t.timescale();
    config.language = t.language().to_string();
    Ok(config)
}

fn writer(path: &Path) -> Result<Mp4Writer<File>> {
    let config = Mp4Config {
        major_brand: "isom".parse()?,
        minor_version: 512,
        compatible_brands: ["isom", "iso2", "avc1", "mp41"]
            .iter().map(|s| s.parse()).collect::<mp4::Result<Vec<_>>>()?,
        timescale: 1000,
    };
    Ok(Mp4Writer::write_start(File::create(path)?, &config)?)
}

fn isolate(source: &Path, target: &Path, codec: MediaType) -> Result<()> {
    let mut input = reader(source)?;
    let (id, config, count) = track(&input, codec)?;
    let mut output = writer(target)?;
    output.add_track(&config)?;
    for index in 1..=count {
        let sample = input.read_sample(id, index)?
            .ok_or_else(|| format!("missing {codec:?} sample {index}"))?;
        output.write_sample(1, &sample)?;
    }
    output.write_end()?;
    println!("RUST_SPLIT_OK codec={codec:?} samples={count} bytes={}", target.metadata()?.len());
    Ok(())
}

fn next(reader: &mut Reader, id: u32, index: &mut u32, count: u32) -> Result<Option<Mp4Sample>> {
    if *index > count {
        return Ok(None);
    }
    let sample = reader.read_sample(id, *index)?
        .ok_or_else(|| format!("missing sample index {}", *index))?;
    *index += 1;
    Ok(Some(sample))
}

fn merge(video: &Path, audio: &Path, output: &Path) -> Result<()> {
    let mut vr = reader(video)?;
    let mut ar = reader(audio)?;
    let (vid, vconfig, vcount) = track(&vr, MediaType::H264)?;
    let (aid, aconfig, acount) = track(&ar, MediaType::AAC)?;
    if vconfig.timescale == 0 || aconfig.timescale == 0 {
        return Err("source track timescale cannot be zero".into());
    }
    let vscale = vconfig.timescale;
    let ascale = aconfig.timescale;
    let mut output_writer = writer(output)?;
    output_writer.add_track(&vconfig)?;
    output_writer.add_track(&aconfig)?;
    let (mut vi, mut ai) = (1, 1);
    let (mut v, mut a) = (
        next(&mut vr, vid, &mut vi, vcount)?,
        next(&mut ar, aid, &mut ai, acount)?,
    );
    // Interleave by decode time without loading the whole movie into memory.
    while v.is_some() || a.is_some() {
        let take_video = match (&v, &a) {
            (Some(v), Some(a)) =>
                u128::from(v.start_time) * u128::from(ascale)
                    <= u128::from(a.start_time) * u128::from(vscale),
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_video {
            output_writer.write_sample(1, v.as_ref().expect("video selected"))?;
            v = next(&mut vr, vid, &mut vi, vcount)?;
        } else {
            output_writer.write_sample(2, a.as_ref().expect("audio selected"))?;
            a = next(&mut ar, aid, &mut ai, acount)?;
        }
    }
    output_writer.write_end()?;
    println!("RUST_REMUX_OK h264_samples={vcount} aac_samples={acount} bytes={}", output.metadata()?.len());
    Ok(())
}

fn verify(original: &Path, result: &Path) -> Result<()> {
    let mut before = reader(original)?;
    let mut after = reader(result)?;
    for codec in [MediaType::H264, MediaType::AAC] {
        let (input_id, _, input_count) = track(&before, codec)?;
        let (output_id, _, output_count) = track(&after, codec)?;
        if input_count == 0 || input_count != output_count {
            return Err(format!("{codec:?} sample count mismatch {input_count}/{output_count}").into());
        }
        for idx in 1..=input_count {
            let x = before.read_sample(input_id, idx)?.ok_or("input sample missing")?;
            let y = after.read_sample(output_id, idx)?.ok_or("output sample missing")?;
            if x.bytes != y.bytes || x.duration != y.duration
                || x.start_time != y.start_time || x.is_sync != y.is_sync
            {
                return Err(format!("{codec:?} compressed sample changed at {idx}").into());
            }
        }
        println!("RUST_SAMPLE_FIDELITY_PASS codec={codec:?} samples={input_count}");
    }
    if after.tracks().len() != 2 {
        return Err("remux output must contain exactly two tracks".into());
    }
    println!("RUST_MP4_REMUX_ACCEPTED encoded_bytes_unchanged=1 codecs=H264,AAC");
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 || args[1] != "prove" {
        return Err("usage: akula-video-lite-lab prove <synthetic.mp4> <scratch-dir> <result.mp4>".into());
    }
    let source = Path::new(&args[2]);
    let scratch = Path::new(&args[3]);
    let result = Path::new(&args[4]);
    std::fs::create_dir_all(scratch)?;
    let video = scratch.join("video-only.mp4");
    let audio = scratch.join("audio-only.m4a");
    isolate(source, &video, MediaType::H264)?;
    isolate(source, &audio, MediaType::AAC)?;
    merge(&video, &audio, result)?;
    verify(source, result)?;
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("RUST_MP4_REMUX_FAILURE: {err}");
        std::process::exit(1);
    }
}
