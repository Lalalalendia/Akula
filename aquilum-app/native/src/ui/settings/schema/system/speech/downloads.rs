use super::*;

pub(super) fn controls(rows: &mut Vec<Row>, view: &View) {
    let (description, action) = if view.auto_downloads {
        ("autoDownloadsOn", "autoDownloadsDisable")
    } else {
        ("autoDownloadsOff", "autoDownloadsEnable")
    };
    rows.push(info(
        label("autoDownloads"),
        Some(label(description)),
        buttons(vec![
            button(action, Speech::AutoDownloads(!view.auto_downloads)).enabled(
                view.loaded && (view.auto_downloads || view.tools.iter().all(Option::is_some)),
            ),
        ]),
    ));
    if let Some(path) = view.admitting_download.as_deref() {
        let text = t_with(
            "settings.system.speech.downloadPreparing",
            &[("name", &file_name(path))],
        );
        rows.push(status("progress", text, Tone::Secondary));
    }
}
