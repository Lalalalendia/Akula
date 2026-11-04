mod startup;
use super::*;
pub fn parse() -> Result<Args, String> {
    let mut args = startup::defaults();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag}: нет значения"));
        match flag.as_str() {
            "--vault" => args.vault = Some(value()?.into()),
            "--data-dir" => args.data_dir = Some(value()?.into()),
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or("--size: нужно ШxВ, например 2881x1717")?;
                let parse = |s: &str| s.parse::<u32>().map_err(|e| format!("--size: {e}"));
                args.size = Some(PhysicalSize::new(parse(w)?, parse(h)?));
            }
            "--file" => {
                let path = value()?;
                args.text = std::fs::read_to_string(&path).map_err(|e| format!("--file {path}: {e}"))?;
                args.restore = false;
            }
            "--threads" => args.threads = value()?.parse().map_err(|e| format!("--threads: {e}"))?,
            "--metrics" => args.metrics = Some(value()?.into()),
            "--trace" => args.trace = true,
            #[cfg(not(feature = "installed"))]
            "--paint-flashing" => args.paint_flashing = true,
            "--focus" => args.focus = true,
            "--keep-position" => args.keep_position = true,
            #[cfg(not(feature = "installed"))]
            "--trace-frames" => args.trace_frames = Some(value()?.into()),
            #[cfg(feature = "memory-probe")]
            "--memory-probe" => args.memory_probe = Some(value()?.into()),
            #[cfg(feature = "memory-probe")]
            "--probe-note" => args.probe_notes.push(value()?.into()),
            "--open" => args.open = Some(value()?.into()),
            #[cfg(feature = "dicom-preview")]
            "--open-dicom" => args.open_dicom = Some(value()?.into()),
            #[cfg(not(feature = "installed"))]
            "--replay-scroll" => args.replay = Some((replay::Kind::Scroll, value()?.into())),
            #[cfg(not(feature = "installed"))]
            "--replay-fling" => args.replay = Some((replay::Kind::Fling, value()?.into())),
            #[cfg(not(feature = "installed"))]
            "--replay-graph" => args.replay = Some((replay::Kind::Graph, value()?.into())),
            #[cfg(not(feature = "installed"))]
            "--replay-focus" => args.replay = Some((replay::Kind::Focus, value()?.into())),
            other => startup::positional(&mut args, other)?,
        }
    }
    #[cfg(feature = "dicom-preview")]
    if args.open.is_some() && args.open_dicom.is_some() {
        return Err("--open и --open-dicom нельзя использовать одновременно".into());
    }
    startup::finish(args)
}
