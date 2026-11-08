//! Optional AK-CALL-04 trace runner: uses the actual feature-gated Rust planner.
//! No capture, network, codec, or OS permissions. Input is raw BGRA8 frames.

#[cfg(not(feature = "screen-damage-lab"))]
fn main() {
    eprintln!("Enable --features screen-damage-lab for this offline example");
    std::process::exit(2);
}

#[cfg(feature = "screen-damage-lab")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use aquilum_core::screen_damage_lab::{
        plan_update, Frame, FrameUpdate, MoveRect, Rect, ScreenReceiver,
        MAX_HEIGHT, MAX_WIDTH,
    };
    use std::fs::File;
    use std::io::{self, BufReader, Read};

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err("usage: ak_call04_damage_metrics MODE WIDTH HEIGHT FRAMES INPUT.bgra".into());
    }
    let scenario = args[1].as_str();
    if !matches!(scenario, "edits" | "scroll" | "motion") {
        return Err("mode must be edits, scroll, or motion".into());
    }
    let width: usize = args[2].parse()?;
    let height: usize = args[3].parse()?;
    let frames: usize = args[4].parse()?;
    if !(64..=MAX_WIDTH).contains(&width)
        || !(64..=MAX_HEIGHT).contains(&height)
        || !(2..=120).contains(&frames)
    {
        return Err("invalid dimensions or frame count".into());
    }
    let frame_bytes = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("frame size overflow")?;
    if frame_bytes.checked_mul(frames).ok_or("trace size overflow")? > 256 * 1024 * 1024 {
        return Err("trace exceeds 256 MiB bound".into());
    }
    let mut reader = BufReader::new(File::open(&args[5])?);
    let mut receiver = ScreenReceiver::new();
    let mut baseline: Option<Frame> = None;
    let mut raw_payload_bytes = 0usize;
    let (mut keyframes, mut deltas, mut unchanged, mut move_count) = (0, 0, 0, 0);

    for id in 1..=frames {
        let mut pixels = vec![0u8; frame_bytes];
        reader.read_exact(&mut pixels)?;
        let target = Frame::new(id as u64, width, height, pixels)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{err:?}")))?;
        // In the scroll trace, the synthetic capture's independently known
        // motion is upward by eight rows. Other traces provide no move hints.
        let moves = if scenario == "scroll" && id > 1 {
            vec![MoveRect {
                source: Rect { x: 0, y: 8, width, height: height - 8 },
                dest_x: 0,
                dest_y: 0,
            }]
        } else {
            Vec::new()
        };
        let update = plan_update(baseline.as_ref(), &target, &moves)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{err:?}")))?;
        raw_payload_bytes = raw_payload_bytes
            .checked_add(update.raw_payload_bytes())
            .ok_or("raw byte count overflow")?;
        match &update {
            FrameUpdate::Keyframe(_) => keyframes += 1,
            FrameUpdate::Delta { moves, .. } => {
                deltas += 1;
                move_count += moves.len();
            }
            FrameUpdate::Unchanged => unchanged += 1,
        }
        receiver.apply(&update)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{err:?}")))?;
        if receiver.frame().ok_or("missing display frame")?.pixels() != target.pixels() {
            return Err(format!("exact reconstruction failed at frame {id}").into());
        }
        if !matches!(update, FrameUpdate::Unchanged) {
            baseline = Some(target);
        }
    }
    let mut tail = [0u8; 1];
    if reader.read(&mut tail)? != 0 {
        return Err("trailing partial or extra raw frames".into());
    }
    println!(
        "scenario={scenario} frames={frames} raw_payload_bytes={raw_payload_bytes} \
         keyframes={keyframes} deltas={deltas} unchanged={unchanged} moves={move_count}"
    );
    Ok(())
}
