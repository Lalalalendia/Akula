use super::*;

#[test]
fn round_trips_both_forms() {
    for line in ["![601|left](<Files/image 6.png>)", "![[file.png|520|right|crop=10,12.5,90,80]]", "![](Files/a.png)"] {
        assert_eq!(format(&parse(line)), line);
    }
    let e = parse("![alt text|300](a.png)");
    assert_eq!(e.params.width, Some(300.0));
    assert_eq!(format(&e), "![300](a.png)");
}

#[test]
fn title_is_not_part_of_the_address() {
    for (line, src) in [("![x](https://e.com/a.png?t=1 \"Подсказка\")", "https://e.com/a.png?t=1"), ("![x](<Files/image 6.png> 'Подсказка')", "Files/image 6.png")] {
        let e = parse(line);
        assert_eq!(e.src, src);
        assert_eq!(format(&e), line.replacen("![x]", "![]", 1), "заголовок потерялся при пересборке строки");
    }
}

#[test]
fn only_decodable_images_and_playable_videos_are_embeds() {
    for line in ["![[a.png]]", "![[a.PNG|300]]", "![[clip.mp4]]", "![[a.png#x|10]]", "![](a.jpg)", "![](https://x.io/photo)", "![alt](<Files/a b.webp> \"t\")"] {
        assert!(is_embed(line), "{line}");
    }
    for line in ["![[a.svg]]", "![[a.avif|200]]", "![[doc.pdf]]", "![[Заметка]]", "![](a.svg)", "![](https://x.io/a.avif?w=1)", "![](a.png", "![](a.pdf)"] {
        assert!(!is_embed(line), "{line}");
    }
}

#[test]
fn width_by_height_and_html_images_keep_their_size() {
    let e = parse("![[a.png|300x200]]");
    assert_eq!((e.params.width, e.params.height), (Some(300.0), Some(200.0)));
    assert_eq!(format(&e), "![[a.png|300x200]]");
    let mut resized = e.params.clone();
    resized.resize(150.0);
    assert_eq!((resized.width, resized.height), (Some(150.0), Some(100.0)));
    let html = parse(r#"<img alt="x" src="Files/a b.png" width="150px" data-src="no" />"#);
    assert_eq!((html.src.as_str(), html.wiki, html.params.width, html.params.height), ("Files/a b.png", false, Some(150.0), None));
    assert!(is_html_image("<IMG src=a.png>") && !is_html_image("<image>x"));
}

#[test]
fn wiki_address_loses_anchor_and_label() {
    let e = parse("![[ a.png # p | 300 | left ]]");
    assert_eq!((e.src.as_str(), e.params.width, e.params.align), ("a.png", Some(300.0), Alignment::Start));
}
