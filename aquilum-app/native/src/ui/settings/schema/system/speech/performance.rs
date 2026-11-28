use super::*;
use aquilum_core::background_jobs::speech::CpuMode;

pub(super) fn row(view: &View) -> Row {
    let description = t_with(
        "settings.system.speech.cpuHint",
        &[("threads", &view.cpu_mode.threads().to_string())],
    );
    info(
        label("cpuMode"),
        Some(description),
        buttons(
            [
                ("cpuQuiet", CpuMode::Quiet),
                ("cpuBalanced", CpuMode::Balanced),
                ("cpuFast", CpuMode::Fast),
            ]
            .into_iter()
            .map(|(key, mode)| {
                button(key, Speech::CpuMode(mode)).enabled(view.loaded && mode != view.cpu_mode)
            })
            .collect(),
        ),
    )
}
