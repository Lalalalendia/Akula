use std::path::PathBuf;
use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(64);
const FLING_STEP: Duration = Duration::from_millis(8);
const FLING_LINES: f32 = 3.0;
const REFRESH_MS: f64 = 1000.0 / 120.0;
const FOCUS_SECONDS: f64 = 1.2;

pub enum Step {
    Wheel(f32),
    Pinch(f32),
    Pan(f32, f32),
    Focus,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Scroll,
    Fling,
    Graph,
    Focus,
}

pub struct Replay {
    kind: Kind,
    length: Duration,
    output: PathBuf,
    started: Option<Instant>,
    next: Option<Instant>,
    presents: Vec<Instant>,
    work: Vec<f64>,
}

impl Replay {
    pub fn new(kind: Kind, output: PathBuf) -> Self {
        let seconds = match kind {
            Kind::Scroll => 4.0,
            Kind::Fling => 6.0,
            Kind::Graph => 20.0,
            Kind::Focus => FOCUS_SECONDS * 2.0,
        };
        Replay { kind, length: Duration::from_secs_f64(seconds), output, started: None, next: None, presents: Vec::new(), work: Vec::new() }
    }

    pub fn is_graph(&self) -> bool {
        self.kind == Kind::Graph
    }

    pub fn step(&mut self, now: Instant) -> Option<Step> {
        match self.kind {
            Kind::Scroll | Kind::Fling => return self.scroll(now).map(Step::Wheel),
            Kind::Focus => return self.focus(now),
            Kind::Graph => {}
        }
        let started = *self.started.get_or_insert(now);
        let next = *self.next.get_or_insert(now);
        if now < next {
            return None;
        }
        let elapsed = now - started;
        if elapsed >= self.length {
            self.next = None;
            return None;
        }
        self.next = Some(next + Duration::from_millis(16));
        let phase = (elapsed.as_secs_f64() % 10.0) / 10.0;
        Some(match phase {
            p if p < 0.35 => Step::Pinch(0.02),
            p if p < 0.65 => Step::Pan(6.0, 3.0),
            _ => Step::Pinch(-0.02),
        })
    }

    fn focus(&mut self, now: Instant) -> Option<Step> {
        let started = *self.started.get_or_insert(now);
        let next = *self.next.get_or_insert(now);
        if now < next {
            return None;
        }
        if now - started >= self.length {
            self.next = None;
            return None;
        }
        self.next = Some(next + Duration::from_secs_f64(FOCUS_SECONDS));
        Some(Step::Focus)
    }

    pub fn wake(&self) -> Option<Instant> {
        self.next
    }

    fn scroll(&mut self, now: Instant) -> Option<f32> {
        let started = *self.started.get_or_insert(now);
        let next = *self.next.get_or_insert(now);
        if now < next {
            return None;
        }
        let elapsed = now - started;
        if elapsed >= self.length {
            self.next = None;
            return None;
        }
        let (step, lines) = if self.kind == Kind::Fling { (FLING_STEP, FLING_LINES) } else { (STEP, 1.0) };
        self.next = Some(next + step);
        Some(if elapsed < self.length / 2 { -lines } else { lines })
    }

    pub fn running(&self) -> bool {
        self.started.is_some()
    }

    pub fn finished(&self) -> bool {
        self.started.is_some() && self.next.is_none()
    }

    pub fn frame(&mut self, presented: Instant, work: Duration) {
        if self.started.is_some() && self.next.is_some() {
            self.presents.push(presented);
            self.work.push(work.as_secs_f64() * 1000.0);
        }
    }

    fn report_json(&self, include_work_samples: bool) -> String {
        let intervals: Vec<f64> = self.presents.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0).collect();
        let span = self.presents.last().zip(self.presents.first()).map_or(0.0, |(l, f)| (*l - *f).as_secs_f64());
        let [p95] = crate::stats::percentiles(&intervals, [0.95]);
        let missed = intervals.iter().filter(|&&i| i > REFRESH_MS * 1.5).count();
        // Rust tuple Debug output such as [(0, 34.0)] is not JSON.
        // Serialize slow-frame entries as genuine arrays; do not lose the
        // very stalls this diagnostic must capture.
        let long = intervals.iter().enumerate()
            .filter(|(_, interval)| **interval > 30.0)
            .map(|(index, interval)| (index, interval.round()))
            .collect::<Vec<_>>();
        let long_json = serde_json::to_string(&long).expect("finite replay interval JSON");
        let json = format!(
            "{{\"quarters\":{:?},\"frames\":{},\"fps\":{:.1},\"intervalAvgMs\":{:.2},\"intervalP95Ms\":{:.2},\"missed\":{},\"workAvgMs\":{:.2},\"long\":{}}}",
            self.work.chunks(self.work.len().div_ceil(4).max(1)).map(|c| (c.iter().sum::<f64>() / c.len() as f64 * 100.0).round() / 100.0).collect::<Vec<_>>(),
            self.presents.len(),
            if span > 0.0 { (self.presents.len().saturating_sub(1)) as f64 / span } else { 0.0 },
            crate::stats::mean(&intervals),
            p95,
            missed,
            crate::stats::mean(&self.work),
            long_json,
        );
        // --replay-scroll records only presented replay frames. Unlike the
        // renderer's --metrics frame samples, this excludes startup.
        // Extra raw samples stay opt-in and do not change the normal schema.
        if include_work_samples {
            let samples = serde_json::to_string(&self.work).expect("finite CPU replay frame times");
            debug_assert!(json.ends_with('}'));
            json.pop();
            json.push_str(",\"workSamplesMs\":");
            json.push_str(&samples);
            json.push('}');
        }
        json
    }

    pub fn write(&self) {
        let json = self.report_json(
            std::env::var("AKULA_EXPERIMENTAL_FRAME_SAMPLES").ok().as_deref() == Some("1"),
        );
        if let Err(error) = std::fs::write(&self.output, json) {
            eprintln!("замер прокрутки не записан: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_only_cpu_samples_match_presented_frames_without_changing_default_json() {
        let mut replay = Replay::new(Kind::Scroll, PathBuf::from("not-used.json"));
        let start = Instant::now();
        assert!(matches!(replay.step(start), Some(Step::Wheel(_))));
        replay.frame(start + Duration::from_millis(2), Duration::from_micros(900));
        replay.frame(start + Duration::from_millis(18), Duration::from_micros(1200));

        let plain: serde_json::Value = serde_json::from_str(&replay.report_json(false)).unwrap();
        let measured: serde_json::Value = serde_json::from_str(&replay.report_json(true)).unwrap();
        assert!(plain.get("workSamplesMs").is_none());
        let samples = measured["workSamplesMs"].as_array().expect("diagnostic samples");
        assert_eq!(samples.len(), replay.presents.len());
        assert_eq!(samples.len(), 2);
        assert!((samples[0].as_f64().unwrap() - 0.9).abs() < 1e-9);
        assert!((samples[1].as_f64().unwrap() - 1.2).abs() < 1e-9);
    }

    #[test]
    fn long_frame_interval_is_valid_json_pair() {
        let dir = tempfile::tempdir().expect("temporary diagnostics");
        let path = dir.path().join("replay.json");
        let mut replay = Replay::new(Kind::Scroll, path.clone());
        let started = Instant::now();
        assert!(matches!(replay.step(started), Some(Step::Wheel(_))));
        replay.frame(started + Duration::from_millis(2), Duration::from_millis(3));
        replay.frame(started + Duration::from_millis(50), Duration::from_millis(3));
        replay.write();
        let parsed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).expect("valid JSON on long frame");
        assert_eq!(parsed["frames"], 2);
        assert_eq!(parsed["long"], serde_json::json!([[0, 48.0]]));
    }
}
