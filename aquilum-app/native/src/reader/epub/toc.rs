use crate::reader::dom::{Dom, NodeId};
use crate::reader::zip::Zip;

use super::{descendants, document, limits};

const MAX_ENTRIES: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub title: String,
    pub section: usize,
    pub fragment: Option<String>,
    pub depth: usize,
}

// Resolve package-relative references without allowing "../" outside the ZIP root.
pub(super) fn join(base: &str, href: &str) -> String {
    let href = crate::paths::percent_decode(href.split('#').next().unwrap_or(href));
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn child(dom: &Dom, node: NodeId, name: &str) -> Option<NodeId> {
    dom.node(node).children.iter().copied().find(|&id| dom.name(id) == Some(name))
}

fn label(dom: &Dom, root: NodeId) -> String {
    let mut stack = vec![root];
    let mut out = String::new();
    while let Some(id) = stack.pop() {
        if let Some(value) = dom.text_of(id) {
            out.push_str(value);
        } else {
            stack.extend(dom.node(id).children.iter().rev().copied());
        }
        if out.len() > 1_024 {
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect()
}

fn target(href: &str, folder: &str, sections: &[(String, String)]) -> Option<(usize, Option<String>)> {
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let section = sections.iter().position(|(file, _)| file == &join(folder, path))?;
    let fragment = crate::paths::percent_decode(fragment);
    Some((section, (!fragment.is_empty()).then_some(fragment)))
}

fn from_nav(dom: &Dom, folder: &str, sections: &[(String, String)]) -> Vec<Entry> {
    let Some(nav) = descendants(dom, Dom::ROOT, "nav").find(|&n| {
        dom.attr(n, "type").is_some_and(|v| v.split_whitespace().any(|s| s == "toc"))
            || dom.attr(n, "role") == Some("doc-toc")
    }) else { return Vec::new() };
    let Some(ol) = child(dom, nav, "ol") else { return Vec::new() };
    let mut stack: Vec<_> = dom.node(ol).children.iter().rev().filter(|&&id| dom.name(id) == Some("li")).map(|&n| (n, 0)).collect();
    let mut entries = Vec::new();
    while let Some((li, depth)) = stack.pop() {
        if entries.len() >= MAX_ENTRIES { break; }
        if let Some(anchor) = child(dom, li, "a")
            && let Some(href) = dom.attr(anchor, "href")
            && let Some((section, fragment)) = target(href, folder, sections)
        {
            let title = label(dom, anchor);
            if !title.is_empty() {
                entries.push(Entry { title, section, fragment, depth });
            }
        }
        if let Some(nested) = child(dom, li, "ol") {
            stack.extend(dom.node(nested).children.iter().rev().filter(|&&n| dom.name(n) == Some("li")).map(|&n| (n, (depth + 1).min(16))));
        }
    }
    entries
}

fn from_ncx(dom: &Dom, folder: &str, sections: &[(String, String)]) -> Vec<Entry> {
    let Some(nav) = descendants(dom, Dom::ROOT, "navMap").next() else { return Vec::new() };
    let mut stack: Vec<_> = dom.node(nav).children.iter().rev().filter(|&&id| dom.name(id) == Some("navPoint")).map(|&n| (n, 0)).collect();
    let mut entries = Vec::new();
    while let Some((node, depth)) = stack.pop() {
        if entries.len() >= MAX_ENTRIES { break; }
        if let Some(content) = child(dom, node, "content")
            && let Some(src) = dom.attr(content, "src")
            && let Some((section, fragment)) = target(src, folder, sections)
            && let Some(nav_label) = child(dom, node, "navLabel")
        {
            let title = label(dom, nav_label);
            if !title.is_empty() {
                entries.push(Entry { title, section, fragment, depth });
            }
        }
        stack.extend(dom.node(node).children.iter().rev().filter(|&&id| dom.name(id) == Some("navPoint")).map(|&n| (n, (depth + 1).min(16))));
    }
    entries
}

pub(super) fn parse(zip: &Zip, opf: &Dom, opf_path: &str, sections: &[(String, String)]) -> Vec<Entry> {
    let folder = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let items: Vec<_> = descendants(opf, Dom::ROOT, "item").collect();
    let mut nav = None;
    let mut ncx = None;
    let spine_toc = descendants(opf, Dom::ROOT, "spine").next().and_then(|id| opf.attr(id, "toc"));
    for id in items {
        let Some(href) = opf.attr(id, "href") else { continue };
        if opf.attr(id, "properties").is_some_and(|p| p.split_whitespace().any(|p| p == "nav")) {
            nav = Some(join(folder, href));
        }
        if (opf.attr(id, "id") == spine_toc && spine_toc.is_some())
            || (ncx.is_none() && opf.attr(id, "media-type") == Some("application/x-dtbncx+xml"))
        {
            ncx = Some(join(folder, href));
        }
    }
    for (path, epub3) in [(nav, true), (ncx, false)] {
        let Some(path) = path else { continue };
        let Some(bytes) = zip.read_limited(&path, limits::MAX_PACKAGE_BYTES) else { continue };
        let Some(dom) = document(&bytes) else { continue };
        let toc_folder = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let entries = if epub3 { from_nav(&dom, toc_folder, sections) } else { from_ncx(&dom, toc_folder, sections) };
        if !entries.is_empty() { return entries; }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::session::epub_acceptance::{synthetic_epub, write_zip};
    use crate::reader::session::{Session, Target};

    #[test]
    fn epub2_ncx_chapters_are_read_in_order_and_navigable() {
        let (_temp, path) = synthetic_epub();
        let mut session = Session::open(&path, Target::Start).unwrap();
        let entries = session.contents();
        assert_eq!(entries.len(), 3);
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(entry.title, format!("Глава {index}"));
            assert_eq!(entry.section, index);
            assert_eq!(entry.depth, 0);
        }
        assert!(session.jump_to_contents(2));
        assert_eq!(session.section(), 2);
        assert!(!session.jump_to_contents(99));
        assert_eq!(session.section(), 2);
    }

    #[test]
    fn epub3_nav_prefers_toc_and_keeps_nested_fragment_targets() {
        let dir = crate::test_dir::tempdir();
        let file = dir.path().join("contents.epub");
        let container = r#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
          <manifest>
            <item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/>
            <item id="ch0" href="chapters/0.xhtml" media-type="application/xhtml+xml"/>
            <item id="ch1" href="chapters/1.xhtml" media-type="application/xhtml+xml"/>
          </manifest><spine><itemref idref="ch0"/><itemref idref="ch1"/></spine></package>"#;
        let nav = r##"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
          <body><nav epub:type="landmarks"><ol><li><a href="chapters/1.xhtml">Not the TOC</a></li></ol></nav>
          <nav epub:type="toc"><ol>
             <li><a href="chapters/0.xhtml">Вступление</a></li>
             <li><a href="chapters/1.xhtml#part">Часть <span>один</span></a><ol>
                <li><a href="chapters/1.xhtml#anchor">Подраздел</a></li>
             </ol></li>
             <li><a href="https://example.org/other.xhtml">External</a></li>
           </ol></nav></body></html>"##;
        let chapter0 = r#"<html><body><h1>Вступление</h1><p>Первый раздел.</p></body></html>"#;
        let chapter1 = r#"<html><body><h1 id="part">Часть 1</h1><p>Текст.</p>
           <p id="anchor">Точный якорь второй главы.</p></body></html>"#;
        let members: Vec<(String, Vec<u8>)> = [
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", container),
            ("OPS/book.opf", opf),
            ("OPS/nav.xhtml", nav),
            ("OPS/chapters/0.xhtml", chapter0),
            ("OPS/chapters/1.xhtml", chapter1),
        ].into_iter().map(|(name, value)| (name.to_owned(), value.as_bytes().to_vec())).collect();
        write_zip(&file, &members);
        let mut session = Session::open(&file, Target::Start).expect("EPUB3 opens");
        assert_eq!(
            session.contents().iter().map(|e| (e.title.as_str(), e.section, e.depth, e.fragment.as_deref())).collect::<Vec<_>>(),
            [("Вступление", 0, 0, None), ("Часть один", 1, 0, Some("part")), ("Подраздел", 1, 1, Some("anchor"))]
        );
        assert!(session.jump_to_contents(2));
        assert_eq!(session.section(), 1);
        let (mut fcx, mut lcx) = (masonry::parley::FontContext::new(), masonry::parley::LayoutContext::new());
        session.ensure(masonry::kurbo::Size::new(800.0, 600.0), &crate::reader::tests::settings(18.0, false), &mut fcx, &mut lcx);
        let saved = session.location().expect("contents destination should yield CFI").0;
        let reopened = Session::open(&file, Target::Cfi(saved)).unwrap();
        assert_eq!(reopened.section(), 1);
    }

    #[test]
    fn package_without_valid_toc_still_opens_without_navigation_entries() {
        let (_temp, path) = synthetic_epub();
        let epub = crate::reader::Book::open(&path).expect("valid book");
        assert_eq!(epub.sections.len(), 3);
    }
}
