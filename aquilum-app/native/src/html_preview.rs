//! Experimental read-only HTML conversion. No network, browser or writes.
//! A bounded worker/process is required before shipping for untrusted HTML.
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_HTML_BYTES: u64 = 2 * 1024 * 1024;
const MAX_VISIBLE_BYTES: usize = 512 * 1024;
const WRAP_COLUMNS: usize = 92;

pub(crate) fn is_html_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}
/// Only ordinary files inside the current vault; never follow a source link.
pub(crate) fn admit(vault: &Path, path: &Path) -> Result<PathBuf, String> {
    if !is_html_path(path) { return Err("Нужен файл HTML или HTM".into()); }
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() {
        return Err("HTML должен быть обычным файлом, а не ссылкой".into());
    }
    if meta.len() > MAX_HTML_BYTES {
        return Err("HTML превышает ограничение 2 МиБ для упрощённого просмотра".into());
    }
    let root = fs::canonicalize(vault).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if !source.starts_with(root) {
        return Err("HTML находится вне открытой базы Akula".into());
    }
    Ok(source)
}
fn render_html(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    // Reject unknown encodings rather than silently corrupt Cyrillic.
    std::str::from_utf8(bytes)
        .map_err(|_| "Пока поддерживается HTML в UTF-8; другая кодировка не преобразована".to_owned())?;
    // html2text uses html5ever internally and emits inert text only.
    let text = html2text::from_read(bytes, WRAP_COLUMNS)
        .map_err(|e| format!("Не удалось прочитать HTML: {e}"))?;
    if text.len() > MAX_VISIBLE_BYTES {
        return Err("HTML содержит слишком много текста для простого просмотра".into());
    }
    Ok(text)
}
pub(crate) fn read(vault: &Path, path: &Path) -> Result<String, String> {
    let source = admit(vault, path)?;
    let mut input = fs::File::open(&source).map_err(|e| e.to_string())?;
    let len = input.metadata().map_err(|e| e.to_string())?.len();
    if len > MAX_HTML_BYTES { return Err("HTML изменился: файл стал слишком большим".into()); }
    let mut bytes = Vec::with_capacity(len as usize);
    input.by_ref().take(MAX_HTML_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != len || bytes.len() as u64 > MAX_HTML_BYTES {
        return Err("HTML изменился во время чтения".into());
    }
    #[cfg(feature = "html-preview-shiva")]
    { return crate::html_preview_shiva::render_trusted(&bytes); }
    #[cfg(not(feature = "html-preview-shiva"))]
    render_html(&bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    const WPT_LIST: &str = include_str!("../tests/fixtures/html/wpt_li_001.html");
    const WPT_SCRIPT: &str = include_str!("../tests/fixtures/html/wpt_script.html");

    #[test]
    fn real_wpt_html_files_render_without_executing_script() {
        let list = render_html(WPT_LIST.as_bytes()).unwrap();
        assert!(list.contains("Description"));
        assert!(list.contains("Unordered List"));
        assert!(list.contains("list item"));
        let script = render_html(WPT_SCRIPT.as_bytes()).unwrap();
        assert!(!script.contains("document.createElement"));
        assert!(!script.contains("assert_false"));
    }
    #[test]
    fn unicode_links_lists_and_malformed_html_are_readable() {
        let html = r#"<!doctype html><title>Пример</title><h1>Акула 海</h1>
        <p>Начало <strong>статьи</strong> &amp; продолжение.
        <ul><li>Первый<li>Второй</ul>
        <a href="https://example.org/path">Ссылка</a>
        <script>window.steal = function() { fetch("/etc/passwd"); }</script>
        <iframe src="https://untrusted.invalid/"></iframe>"#;
        let text = render_html(html.as_bytes()).unwrap();
        assert!(text.contains("Акула 海"));
        assert!(text.contains("Начало"));
        assert!(text.contains("Первый") && text.contains("Второй"));
        assert!(text.contains("Ссылка"));
        assert!(!text.contains("window.steal"));
        assert!(!text.contains("fetch("));
    }

    #[cfg(not(feature = "html-preview-shiva"))]
    #[test]
    fn bounded_read_does_not_modify_original() {
        let vault = tempfile::tempdir().unwrap();
        let file = vault.path().join("Страница.HTML");
        fs::write(&file, WPT_LIST.as_bytes()).unwrap();
        let original = fs::read(&file).unwrap();
        assert!(read(vault.path(), &file).unwrap().contains("Description"));
        assert_eq!(fs::read(&file).unwrap(), original);
        assert!(is_html_path(&file));
        assert!(is_html_path(Path::new("page.hTm")));
        assert!(!is_html_path(Path::new("page.txt")));
    }

    #[test]
    fn reject_outside_symlinks_oversize_and_bad_encodings() {
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("outside.html");
        fs::write(&file, b"<p>Outside</p>").unwrap();
        assert!(admit(vault.path(), &file).is_err());
        let huge = vault.path().join("huge.htm");
        fs::File::create(&huge).unwrap().set_len(MAX_HTML_BYTES + 1).unwrap();
        assert!(admit(vault.path(), &huge).is_err());
        assert!(render_html(b"<p>\xff</p>").unwrap_err().contains("UTF-8"));
        #[cfg(unix)] {
            let link = vault.path().join("link.html");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(admit(vault.path(), &link).is_err());
        }
    }

    // The HTML5 parser-only stage and parse+text-formatting are different work.
    #[test]
    #[ignore = "diagnostic HTML5 parser versus renderer on WPT files"]
    fn wpt_html_parse_vs_text_render_diagnostic() {
        use std::hint::black_box;
        use std::time::Instant;
        for (name, html) in [("wpt-li", WPT_LIST), ("wpt-script", WPT_SCRIPT)] {
            let mut parser = Vec::new();
            let mut rendered = Vec::new();
            for i in 0..40 {
                let parse = || {
                    let begin = Instant::now();
                    black_box(html2text::parse(black_box(html.as_bytes())).unwrap());
                    begin.elapsed().as_micros()
                };
                let format = || {
                    let begin = Instant::now();
                    black_box(render_html(black_box(html.as_bytes())).unwrap());
                    begin.elapsed().as_micros()
                };
                if i % 2 == 0 { parser.push(parse()); rendered.push(format()); }
                else { rendered.push(format()); parser.push(parse()); }
            }
            parser.sort_unstable();
            rendered.sort_unstable();
            eprintln!("AKULA_HTML_WPT_COMPARISON source={name} bytes={} rounds=40 html5ever_parse_only_p50_us={} html2text_parse_and_format_p50_us={} semantics=distinct", html.len(), parser[20], rendered[20]);
        }
    }
}
