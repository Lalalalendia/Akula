use std::path::PathBuf;
use winit::dpi::PhysicalSize;
mod w4_pilot;
pub use w4_pilot::run_w4_pilot_if_requested;
#[cfg(not(feature = "installed"))]
use crate::replay;

pub struct Args {
    pub vault: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub size: Option<PhysicalSize<u32>>,
    pub text: String,
    pub threads: u16,
    pub metrics: Option<PathBuf>,
    pub trace: bool,
    pub paint_flashing: bool,
    pub restore: bool,
    pub keep_position: bool,
    pub focus: bool,
    #[cfg(not(feature = "installed"))]
    pub replay: Option<(replay::Kind, PathBuf)>,
    pub open: Option<PathBuf>,
    #[cfg(feature = "dicom-preview")]
    pub open_dicom: Option<PathBuf>,
    #[cfg(not(feature = "installed"))]
    pub trace_frames: Option<PathBuf>,
    #[cfg(feature = "memory-probe")]
    pub memory_probe: Option<PathBuf>,
    #[cfg(feature = "memory-probe")]
    pub probe_notes: Vec<PathBuf>,
}

mod parse;
pub use parse::parse;
