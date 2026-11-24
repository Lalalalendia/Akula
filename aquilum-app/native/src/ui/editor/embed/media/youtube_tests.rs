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

#[test]
fn standalone_markdown_youtube_links_keep_the_exact_video_target() {
    let url = "https://www.youtube.com/watch?v=YopvaTJsSNA";
    for line in [format!("[www.youtube.com]({url})"), format!("[Видео]({url})"), format!(" [Watch]({url}) ")] {
        assert!(is_embed(&line), "{line}");
        let block = parse(&line);
        assert_eq!(block.src, url);
        assert!(!block.wiki);
        assert_eq!(block.params.width, None);
    }
    for line in [
        "[watch](https://youtu.be/dQw4w9WgXcQ)",
        "[short](https://youtube.com/shorts/dQw4w9WgXcQ)",
    ] { assert!(is_embed(line)); }
    for line in [
        format!("Read [watch]({url})"),
        format!("[watch]({url}) then read"),
        format!("[watch]({url}) [other](https://example.org)"),
        "[watch](https://youtube.com.evil.example/watch?v=YopvaTJsSNA)".into(),
        "[watch](https://youtube.com@evil.example/watch?v=YopvaTJsSNA)".into(),
        "[watch](https://example.org/image.png)".into(),
        "[watch](https://youtu.be/invalid)".into(),
        "[watch](https://www.youtube.com/playlist?list=PLTEST)".into(),
        "[watch](https://youtu.be/dQw4w9WgXcQ".into(),
    ] { assert!(!is_embed(&line), "{line}"); }
}
