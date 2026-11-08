#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub folder: bool,
    pub depth: usize,
    pub expanded: bool,
    pub color: Option<&'static str>,
    pub items: Option<u32>,
    pub guides: Vec<usize>,
    pub marks: Vec<(usize, usize)>,
    pub active: bool,
    pub selected: bool,
}
