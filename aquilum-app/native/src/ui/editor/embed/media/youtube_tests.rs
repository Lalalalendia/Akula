use super::{is_embed, parse};

#[test]
fn standalone_video_urls_are_native_media_without_markdown_rewrite() {
    for line in [
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "https://youtu.be/dQw4w9WgXcQ?t=14",
    ] {
        assert!(is_embed(line));
        let block = parse(line);
        assert_eq!(block.src, line);
        assert!(!block.wiki);
    }
    for line in [
        "Read https://youtu.be/dQw4w9WgXcQ",
        "https://youtu.be/not-valid",
        "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
    ] {
        assert!(!is_embed(line), "{line}");
    }
}
