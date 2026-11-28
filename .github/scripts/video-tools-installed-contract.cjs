'use strict';

const installed = 'all(windows, feature = "installed", not(feature = "video-ui-e2e"))';
const queueGate = '#[cfg(' + installed + ')]';
const launchGate = 'if cfg!(' + installed + ')';
const paths = {
  queue: 'aquilum-app/native/src/video/queue.rs',
  dispatch: 'aquilum-app/native/src/video/queue_dispatch.rs',
  leaf: 'aquilum-app/native/src/video/light.rs'
};

function checkInstalledRoute({queue, dispatch, leaf}, hold) {
  for (const declaration of [
    'let executable = super::optional_ytdlp::verified_executable(&data_dir)',
    'let runtime: Result<Option<OsString>, String> = super::optional_ytdlp::quickjs::verified_executable(&data_dir)'
  ]) {
    const verification = queue.indexOf(queueGate + '\n        ' + declaration);
    hold(verification >= 0 && verification < queue.indexOf('Library::new('),
      'installed queue must verify both opt-in tools before creating local storage');
  }
  hold(queue.includes('#[cfg(not(' + installed + '))]\n        let executable: Result<OsString, String> = Ok(std::env::var_os(') &&
    queue.includes('executable.and_then(|executable| runtime.and_then(|runtime|') &&
    queue.includes('super::queue_dispatch::download_one(&library, &executable, runtime.as_deref(),'),
    'only the non-installed path may select a downloader from the environment');
  hold(dispatch.includes(queueGate + '\n    {') &&
    dispatch.includes('let runtime = quickjs.ok_or(') &&
    dispatch.includes('super::download::download_one_with_js(library, executable, runtime, url, on_progress)') &&
    dispatch.includes('#[cfg(not(' + installed + '))]\n    {\n        let _ = quickjs;\n        super::light::download_one_or_legacy('),
    'ordinary installed Akula must route exclusively to verified opt-in tools');
  const legacy = leaf.split('pub(super) fn download_one_or_legacy(', 2)[1] ?? '';
  hold(legacy.includes(launchGate + ' {\n        return Err("VIDEO_TOOLS_01K_HOLD:') &&
    legacy.indexOf(launchGate) < legacy.indexOf('std::env::var_os("AKULA_RUST_MUX_BIN")'),
    'legacy bundled process launch must refuse ordinary installed Windows');
}

module.exports = {paths, checkInstalledRoute};
