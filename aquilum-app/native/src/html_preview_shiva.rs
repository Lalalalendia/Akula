//! Optional non-shipping HTML adapter to the independently confined Shiva worker.
//! Never fall back to html2text in the GUI when this feature is enabled.
use std::path::PathBuf;
use std::time::Duration;
use shiva_worker_client::{WorkerSettings, extract};

const MAX_INPUT: usize = 2 * 1024 * 1024;
const MAX_TEXT: usize = 512 * 1024;
const SOURCE_ID: &str = "akula_html_preview";

fn trusted_settings() -> Result<WorkerSettings, String> {
    // Set ONLY by a reviewed, trusted build of the installed app; never from
    // a document, runtime environment, user preference or a mutable vault.
    // An absent pin is an error. No direct/in-process parser fallback.
    let pin = option_env!("AKULA_SHIVA_WORKER_BLAKE3")
        .ok_or("Изолированный обработчик HTML не установлен")?;
    let exe = std::env::current_exe().map_err(|_| "Не найдена установленная Акула")?;
    let worker = exe.with_file_name(if cfg!(windows) { "shiva-worker.exe" } else { "shiva-worker" });
    Ok(WorkerSettings {
        executable: worker,
        expected_binary_blake3: pin.to_owned(),
        expected_version: "0.1.0".to_owned(),
        timeout: Duration::from_secs(6),
    })
}

/// This function only accepts already-authorized, bounded in-memory bytes.
fn render_verified(settings: &WorkerSettings, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_INPUT { return Err("HTML превышает лимит 2 МиБ".into()); }
    std::str::from_utf8(bytes).map_err(|_| "HTML должен быть в UTF-8")?;
    let result = extract(settings, SOURCE_ID, "html", bytes)
        .map_err(|error| format!("Изолированное чтение HTML отклонено: {error:?}"))?;
    if result.source.source_id != SOURCE_ID || result.source.format != "html"
        || result.source.input_bytes != bytes.len() as u64 {
        return Err("Ответ обработчика не соответствует исходному HTML".into());
    }
    let mut content = String::new();
    for block in result.blocks {
        let len = content.len().checked_add(block.text.len())
            .and_then(|v| v.checked_add(1))
            .ok_or("Слишком много текста в HTML")?;
        if len > MAX_TEXT { return Err("Текст HTML превышает лимит 512 КиБ".into()); }
        if !content.is_empty() { content.push('\n'); }
        content.push_str(&block.text);
    }
    if content.is_empty() { return Err("Из HTML не удалось получить читаемый текст".into()); }
    Ok(content)
}

pub(crate) fn render_trusted(bytes: &[u8]) -> Result<String, String> {
    render_verified(&trusted_settings()?, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> WorkerSettings {
        // Diagnostic CI only; this is NOT the immutable installer provenance.
        let worker = PathBuf::from(std::env::var_os("AKULA_TEST_SHIVA_WORKER_BIN")
            .expect("CI must build the exact Shiva worker before tests"));
        WorkerSettings {
            expected_binary_blake3: shiva_worker_client::binary_blake3(&worker)
                .expect("read exact test-only worker"),
            executable: worker,
            expected_version: "0.1.0".into(),
            timeout: Duration::from_secs(6),
        }
    }
    #[test]
    fn shiva_html_produces_unicode_without_paths() {
        let input = "<html><h1>Акула 世界</h1><p>Verified HTML text 813</p></html>";
        let output = render_verified(&fixture(), input.as_bytes()).unwrap();
        assert!(output.contains("Акула"));
        assert!(output.contains("Verified HTML text 813"));
        assert!(!output.contains("/tmp/"));
    }
    #[test]
    fn shiva_html_blocks_untrusted_external_image() {
        let html = b"<h1>Heading</h1><img src='file:///etc/passwd' alt='private'>";
        assert!(render_verified(&fixture(), html).is_err());
    }
    #[test]
    fn shiva_html_rejects_large_input_before_launch() {
        let html = vec![b'x'; MAX_INPUT + 1];
        assert!(render_verified(&fixture(), &html).is_err());
    }
    #[test]
    fn shiva_html_rejects_missing_trusted_worker_pin() {
        // The CI build intentionally has no signed release pin.
        if option_env!("AKULA_SHIVA_WORKER_BLAKE3").is_none() {
            assert!(render_trusted(b"<h1>Document</h1>").is_err());
        }
    }
}
