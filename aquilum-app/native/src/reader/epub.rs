use std::collections::HashMap;

use super::cfi::{Cfi, element_path};
use super::dom::{Dom, NodeId};
use super::text::{decode, numeric_entities, parse_xml};
use super::zip::Zip;
mod limits;
mod toc;
pub(crate) use toc::Entry as TocEntry;
pub struct Epub {
    zip: Zip,
    pub sections: Vec<(String, String)>,
    pub toc: Vec<TocEntry>,
}

fn descendants<'a>(dom: &'a Dom, root: NodeId, name: &'a str) -> impl Iterator<Item = NodeId> + 'a {
    let mut stack = vec![root];
    std::iter::from_fn(move || {
        while let Some(node) = stack.pop() {
            stack.extend(dom.node(node).children.iter().rev().copied());
            if dom.name(node) == Some(name) {
                return Some(node);
            }
        }
        None
    })
}

pub fn document(bytes: &[u8]) -> Option<Dom> {
    let text = decode(bytes);
    if let Some(doc) = parse_xml(&text) {
        return Some(Dom::from_xml(&doc));
    }
    let fixed = numeric_entities(&text);
    parse_xml(&fixed).map(|doc| Dom::from_xml(&doc))
}

impl Epub {
    pub fn open(data: Vec<u8>) -> Option<Epub> {
        let zip = Zip::new(data)?;
        let container = document(&zip.read_limited("META-INF/container.xml", limits::MAX_PACKAGE_BYTES)?)?;
        let rootfile = descendants(&container, Dom::ROOT, "rootfile").next()?;
        let opf_path = container.attr(rootfile, "full-path")?.to_owned();
        let opf = document(&zip.read_limited(&opf_path, limits::MAX_PACKAGE_BYTES)?)?;
        let folder = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
        let manifest: HashMap<&str, &str> = descendants(&opf, Dom::ROOT, "item").filter_map(|item| Some((opf.attr(item, "id")?, opf.attr(item, "href")?))).collect();
        let spine = descendants(&opf, Dom::ROOT, "spine").next()?;
        let sections = opf
            .node(spine)
            .children
            .iter()
            .copied()
            .filter(|&c| opf.name(c) == Some("itemref"))
            .map(|itemref| {
                let href = opf.attr(itemref, "idref").and_then(|id| manifest.get(id)).map_or(String::new(), |href| toc::join(&folder, href));
                (href, Cfi::Point(vec![element_path(&opf, itemref)]).to_string())
            })
            .collect::<Vec<_>>();
        if !limits::valid_spine(&zip, &sections) { return None; }
        let toc = toc::parse(&zip, &opf, &opf_path, &sections);
        Some(Epub { zip, sections, toc })
    }

    pub fn size(&self, index: usize) -> usize {
        self.sections.get(index).and_then(|(path, _)| self.zip.size(path)).unwrap_or(0)
    }

    pub fn resource(&self, index: usize, src: &str) -> Option<Vec<u8>> {
        let (path, _) = self.sections.get(index)?;
        let folder = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        self.zip.read_limited(&toc::join(folder, src), limits::MAX_RESOURCE_BYTES)
    }

    pub fn section(&self, index: usize) -> Option<Dom> {
        let (path, _) = self.sections.get(index)?;
        document(&self.zip.read_limited(path, limits::MAX_SECTION_BYTES)?)
    }
}
