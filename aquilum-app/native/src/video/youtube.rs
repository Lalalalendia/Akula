//! Recognise a single YouTube video without interpreting an arbitrary URL.
//! Never pass untrusted hosts to the preview downloader or external process.
pub fn video_id(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.len() > 2048 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let rest = url.strip_prefix("https://")?;
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..host_end].to_ascii_lowercase();
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).unwrap_or(&host);
    let (path, query) = rest[host_end..].split_once('?').unwrap_or((&rest[host_end..], ""));
    let path = path.split('#').next().unwrap_or(path);
    let id = match host {
        "youtu.be" => path.strip_prefix('/')?.split('/').next()?,
        "youtube.com" | "youtube-nocookie.com" => {
            if path == "/watch" {
                query.split('#').next()?.split('&').filter_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == "v").then_some(value)
                }).next()?
            } else {
                let section = path.strip_prefix('/')?;
                let (kind, rest) = section.split_once('/')?;
                if !matches!(kind, "shorts" | "live" | "embed" | "v") {
                    return None;
                }
                rest.split('/').next()?
            }
        }
        _ => return None,
    };
    (id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')).then_some(id)
}

/// A preview image always goes to the fixed YouTube CDN, never a host from
/// the pasted URL. This is image metadata, not an embedded web document.
pub fn thumbnail_url(id: &str) -> Option<String> {
    valid_id(id).then(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
}

fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_single_video_urls() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?list=PLID&v=dQw4w9WgXcQ&t=14",
            "https://youtu.be/dQw4w9WgXcQ?t=30",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://youtube.com/live/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(video_id(url), Some("dQw4w9WgXcQ"), "{url}");
        }
        assert_eq!(thumbnail_url("dQw4w9WgXcQ").as_deref(), Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"));
    }

    #[test]
    fn reject_playlist_hosts_tricks_and_nonvideo_content() {
        for url in [
            "http://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://evil.example/?redirect=https://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com@evil.example/watch?v=dQw4w9WgXcQ",
            "https://youtube.com:8080/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/playlist?list=PLTEST",
            "https://youtube.com/watch?v=bad",
            "https://youtube.com/watch?v=dQw4w9WgXcQ%2F",
            "https://youtube.com/watch?v=dQw4w9WgXcQ\n--exec",
            "https://youtu.be/",
            "https://youtu.be/dQw4w9WgXcQ_extra",
            "https://www.youtube.com/results?search_query=videos",
            "https://youtube.com/shorts/dQw4w9WgXcQ/edit",
        ] {
            assert!(video_id(url).is_none(), "{url:?}");
        }
        assert!(thumbnail_url("../other_dir").is_none());
    }
}
