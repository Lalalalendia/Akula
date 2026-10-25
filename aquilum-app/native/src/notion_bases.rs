//! QA-only native Akula Bases explorer over an independent Notion bundle.
//! All disk verification and File Gate CAS writes happen off the GUI thread.
//! No network access, credentials, computed Notion views, or source-file writes.
use aquilum_core::{Core, notion_api_probe::{
    bases_model,
    bases_edits::{self, Change, EffectiveBase, EffectiveValue, LocalValue},
}};
use std::{fs, path::{Path, PathBuf}, sync::{Arc, mpsc::{self, Sender, Receiver}}};

mod editor;
mod schema_editor;
mod grid;
pub use grid::SelectedCell;
pub use editor::{DraftView, EditKind};
pub use schema_editor::{SchemaDraftView, SchemaMode};

const ROWS: usize = 10;
const COLUMNS: usize = 6;
const MAX_BUNDLES: usize = 32;
const MAX_BASES: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage { #[default] Idle, Loading, Ready, Saving, NeedsReview }
#[derive(Clone, Debug)]
pub struct BaseChoice {
    pub digest: String,
    pub base_id: String,
    pub label: String,
}
#[derive(Clone, Debug)]
pub struct ColumnView {
    pub local_only: bool,
    pub name: String,
    pub property_id: String,
    pub kind: String,
}
#[derive(Clone, Debug)]
pub struct CellView {
    pub row_id: String,
    pub property_id: String,
    pub text: String,
    pub toggle_to: Option<bool>,
    pub revert: bool,
    pub edit_text: bool,
}
#[derive(Clone, Debug)]
pub struct RowView {
    pub row_id: String,
    pub label: String,
    pub cells: Vec<CellView>,
}
#[derive(Clone, Debug, Default)]
pub struct BasesView {
    pub source: Option<String>,
    pub target: Option<PathBuf>,
    pub stage: Stage,
    pub busy: bool,
    pub choices: Vec<BaseChoice>,
    pub base_id: Option<String>,
    pub digest: Option<String>,
    pub columns: Vec<ColumnView>,
    pub rows: Vec<RowView>,
    pub total_rows: usize,
    pub total_columns: usize,
    pub row_start: usize,
    pub column_start: usize,
    pub local_revision: u64,
    pub saved_source_views: usize,
    pub notice: Option<&'static str>,
    pub draft: Option<DraftView>,
    pub schema_draft: Option<SchemaDraftView>,
    pub selected: Option<SelectedCell>,
}
enum Payload {
    Catalog(Vec<BaseChoice>),
    Loaded(EffectiveBase, Option<String>),
}
struct Completion {
    generation: u64,
    source: String,
    target: PathBuf,
    result: Result<Payload, ()>,
}
pub struct BasesSession {
    generation: u64,
    view: BasesView,
    effective: Option<EffectiveBase>,
    disk_hash: Option<String>,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
fn short(s: &str, limit: usize) -> String {
    let text = s.chars().filter(|c| !c.is_control()).take(limit + 1).collect::<String>();
    if text.chars().count() > limit {
        format!("{}…", text.chars().take(limit).collect::<String>())
    } else { text }
}
fn source_text(value: &serde_json::Value) -> String {
    let kind = value.get("type").and_then(serde_json::Value::as_str).unwrap_or("unknown");
    match kind {
        "title" | "rich_text" => {
            let text = value.get(kind).and_then(serde_json::Value::as_array)
                .map(|items| items.iter().filter_map(|item| item.get("plain_text")
                    .and_then(serde_json::Value::as_str)).collect::<String>())
                .unwrap_or_default();
            short(&text, 64)
        }
        "number" => value.get("number").filter(|v| !v.is_null())
            .map(|v| v.to_string()).unwrap_or_default(),
        "checkbox" => if value.get("checkbox").and_then(serde_json::Value::as_bool)==Some(true)
            { "Да".into() } else { "Нет".into() },
        "url" | "email" | "phone_number" => value.get(kind)
            .and_then(serde_json::Value::as_str).map(|s|short(s,64)).unwrap_or_default(),
        "select" => value.get("select").and_then(|s|s.get("name"))
            .and_then(serde_json::Value::as_str).map(|s|short(s,64)).unwrap_or_default(),
        "multi_select" => value.get("multi_select").and_then(serde_json::Value::as_array)
            .map(|xs|short(&xs.iter().filter_map(|x|x.get("name")
                .and_then(serde_json::Value::as_str)).collect::<Vec<_>>().join(", "),64))
            .unwrap_or_default(),
        "relation" => format!("Связей: {}", value.get("relation")
            .and_then(serde_json::Value::as_array).map_or(0,Vec::len)),
        _ => format!("{} — снимок Notion", short(kind,24)),
    }
}
fn label(value: &EffectiveValue) -> String {
    match value {
        EffectiveValue::Source(raw) => source_text(raw),
        EffectiveValue::Local(LocalValue::Text(s)) => short(s,64),
        EffectiveValue::Local(LocalValue::Number(n)) => n.to_string(),
        EffectiveValue::Local(LocalValue::Checkbox(b)) =>
            if *b { "Да (лок.)".into() } else { "Нет (лок.)".into() },
        EffectiveValue::Local(LocalValue::Select(s)) => short(s,64),
        EffectiveValue::Local(LocalValue::MultiSelect(xs)) => short(&xs.join(", "),64),
        EffectiveValue::Local(LocalValue::Relation(xs)) =>
            format!("Связей: {} (лок.)", xs.len()),
        EffectiveValue::EmptyLocal => "—".into(),
    }
}
fn checkbox(value: &EffectiveValue) -> Option<bool> {
    match value {
        EffectiveValue::Source(raw) if raw.get("type").and_then(serde_json::Value::as_str)
            == Some("checkbox") => raw.get("checkbox").and_then(serde_json::Value::as_bool),
        EffectiveValue::Local(LocalValue::Checkbox(b)) => Some(*b),
        EffectiveValue::EmptyLocal => Some(false),
        _ => None,
    }
}
fn catalog(workspace: &Path, source: &str) -> Result<Vec<BaseChoice>, ()> {
    if uuid::Uuid::parse_str(source).map(|v|v.to_string()).ok().as_deref()!=Some(source) {
        return Err(());
    }
    let folder = workspace.join("Notion-import");
    let metadata = fs::symlink_metadata(&folder).map_err(|_| ())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() { return Err(()); }
    let mut found = Vec::new();
    let mut bundles = 0usize;
    for entry in fs::read_dir(&folder).map_err(|_| ())? {
        let entry = entry.map_err(|_| ())?;
        let filename = entry.file_name();
        let filename = filename.to_str().ok_or(())?;
        let Some(digest) = filename.strip_prefix(&format!("{source}-"))
            .and_then(|s|s.strip_suffix(".notion.json")) else { continue; };
        if digest.len()!=64 || !digest.bytes().all(|b|b.is_ascii_hexdigit()
            && !b.is_ascii_uppercase()) { return Err(()); }
        if bundles>=MAX_BUNDLES { return Err(()); }
        bundles+=1;
        if entry.file_type().map_err(|_| ())?.is_symlink() { return Err(()); }
        let model = bases_model::from_independent_bundle(&entry.path()).map_err(|_| ())?;
        if model.source_root != source || model.source_digest != digest || model.needs_review() {
            return Err(());
        }
        for base in model.bases.iter().filter(|b| b.schema_available) {
            if found.len()>=MAX_BASES { return Err(()); }
            found.push(BaseChoice {
                digest: digest.into(), base_id: base.akula_id.clone(),
                label: short(base.display_name.as_deref()
                    .unwrap_or(&base.notion_id),64),
            });
        }
    }
    found.sort_by(|a,b|(&a.label,&a.digest,&a.base_id)
        .cmp(&(&b.label,&b.digest,&b.base_id)));
    Ok(found)
}
fn load(workspace: &Path, source: &str, digest: &str, base_id: &str)
    -> Result<Payload, ()>
{
    // Read the CAS sidecar on BOTH sides of the effective projection.
    // A newer disk hash must never authorize writing from an older UI view.
    let fingerprint=||->Result<Option<(u64,String)>,()> {
        Ok(bases_edits::open(workspace,source,digest).map_err(|_| ())?
            .map(|state|(state.state.revision,state.expected_disk_hash)))
    };
    let before=fingerprint()?;
    let base=bases_edits::read_effective_base(workspace,source,digest,base_id)
        .map_err(|_| ())?;
    let after=fingerprint()?;
    if before!=after || base.local_revision!=before.as_ref().map_or(0,|v|v.0){
        return Err(());
    }
    Ok(Payload::Loaded(base,before.map(|v|v.1)))
}
impl BasesSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx,rx)=mpsc::channel();
        Self { generation:0,view:BasesView::default(),effective:None,
            disk_hash:None,tx,rx,wake }
    }
    pub fn view(&self) -> BasesView {
        let mut out=self.view.clone();
        if let Some(base)=&self.effective {
            out.total_rows=base.rows.len();
            out.total_columns=base.columns.len();
            out.local_revision=base.local_revision;
            out.saved_source_views=base.saved_source_views;
            out.columns=base.columns.iter().skip(out.column_start).take(COLUMNS)
                .map(|c|ColumnView{
                    name:short(&c.name,40),local_only:c.local_only,
                    property_id:c.property_id.clone(),
                    kind:c.kind.clone(),
                }).collect();
            for row in base.rows.iter().skip(out.row_start).take(ROWS) {
                let title=row.cells.iter().find_map(|c|match &c.value {
                    EffectiveValue::Source(raw) if raw.get("type")
                        .and_then(serde_json::Value::as_str)==Some("title")=>
                            Some(source_text(raw)),
                    EffectiveValue::Local(LocalValue::Text(v)) if
                        base.columns.iter().any(|col|col.property_id==c.property_id
                            && col.kind=="title") => Some(short(v,64)),
                    _=>None,
                }).filter(|s|!s.is_empty()).unwrap_or_else(||short(&row.notion_page_id,8));
                let mut cells=Vec::new();
                for col in base.columns.iter().skip(out.column_start).take(COLUMNS) {
                    let cell=row.cells.iter().find(|c|c.property_id==col.property_id);
                    let current=cell.map(|c|&c.value);
                    let can_toggle=col.editable_locally && col.kind=="checkbox";
                    cells.push(CellView{
                        row_id:row.akula_id.clone(),property_id:col.property_id.clone(),
                        text:current.map(label).unwrap_or_else(||"—".into()),
                        toggle_to:if can_toggle { current.and_then(checkbox).map(|b|!b) } else { None },
                        revert:matches!(current,Some(EffectiveValue::Local(_))),
                        edit_text:col.editable_locally && current.is_some() &&
                            matches!(col.kind.as_str(),"title"|"rich_text"|"url"| "email"| "phone_number"|"number"),
                    });
                }
                out.rows.push(RowView{row_id:row.akula_id.clone(),
                    label:title,cells});
            }
        }
        out
    }
    pub fn reset(&mut self) {
        self.generation=self.generation.wrapping_add(1);
        self.view=BasesView::default(); self.effective=None; self.disk_hash=None;
    }
    fn start(&mut self, stage:Stage, work:impl FnOnce()->Result<Payload,()>+Send+'static)
        -> Result<(), ()>
    {
        if self.view.busy {return Err(());}
        self.generation=self.generation.checked_add(1).ok_or(())?;
        let generation=self.generation;
        let source=self.view.source.clone().ok_or(())?;
        let target=self.view.target.clone().ok_or(())?;
        let tx=self.tx.clone();
        let wake=Arc::clone(&self.wake);
        std::thread::Builder::new().name("akula-bases-qa".into())
            .spawn(move || {
                let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                    .unwrap_or(Err(()));
                if tx.send(Completion{generation,source,target,result}).is_ok(){wake();}
            }).map_err(|_| ())?;
        self.view.stage=stage; self.view.busy=true; self.view.notice=None;
        Ok(())
    }
    pub fn open(&mut self, target:PathBuf, source:String) -> Result<(),()> {
        if self.view.draft.is_some() || self.view.schema_draft.is_some() { return Err(()); }
        self.reset();
        self.view.source=Some(source.clone());self.view.target=Some(target.clone());
        self.start(Stage::Loading,move ||catalog(&target,&source).map(Payload::Catalog))
    }
    pub fn select(&mut self, digest:String, base_id:String)->Result<(),()>{
        if self.view.busy || self.view.draft.is_some()
            || self.view.schema_draft.is_some() || !self.view.choices.iter()
            .any(|b|b.digest==digest&&b.base_id==base_id) {return Err(());}
        let source=self.view.source.clone().ok_or(())?;
        let target=self.view.target.clone().ok_or(())?;
        self.view.digest=Some(digest.clone());self.view.base_id=Some(base_id.clone());
        self.view.row_start=0;self.view.column_start=0;self.view.selected=None;
        self.effective=None;self.disk_hash=None;
        self.start(Stage::Loading,move ||load(&target,&source,&digest,&base_id))
    }
    pub fn page(&mut self, row_start:usize,column_start:usize)->bool{
        if self.view.busy || self.view.stage!=Stage::Ready || self.view.draft.is_some()
            || self.view.schema_draft.is_some() {return false;}
        let Some(base)=&self.effective else{return false};
        if row_start%ROWS!=0 || column_start%COLUMNS!=0
            || (row_start>0 && row_start>=base.rows.len())
            || (column_start>0 && column_start>=base.columns.len()){return false;}
        self.view.row_start=row_start;self.view.column_start=column_start;
        self.view.selected=None;
        true
    }
    pub fn reload(&mut self)->Result<(),()>{
        if self.view.busy || self.view.draft.as_ref().is_some_and(|d|!d.frozen)
            || self.view.schema_draft.as_ref().is_some_and(|d|!d.frozen) {return Err(());}
        let source=self.view.source.clone().ok_or(())?;
        let target=self.view.target.clone().ok_or(())?;
        let digest=self.view.digest.clone().ok_or(())?;
        let base=self.view.base_id.clone().ok_or(())?;
        self.effective=None; self.disk_hash=None;self.view.selected=None;
        self.start(Stage::Loading,move ||load(&target,&source,&digest,&base))
    }
    pub fn change(&mut self, core:Arc<Core>, row:&str, property:&str,
        revert:bool)->Result<(),()>
    {
        if self.view.busy || self.view.stage!=Stage::Ready || self.view.draft.is_some()
            || self.view.schema_draft.is_some() {return Err(());}
        let data=self.effective.as_ref().ok_or(())?;
        let column=data.columns.iter().find(|c|c.property_id==property).ok_or(())?;
        let row_data=data.rows.iter().find(|r|r.akula_id==row).ok_or(())?;
        let current=row_data.cells.iter().find(|c|c.property_id==property)
            .map(|c|&c.value).ok_or(())?;
        let cmd=if revert {
            if !matches!(current,EffectiveValue::Local(_)){return Err(());}
            Change::Revert{row_id:row.into(),property_id:property.into()}
        } else {
            if !column.editable_locally || column.kind!="checkbox" {return Err(());}
            let next=!checkbox(current).ok_or(())?;
            Change::Set{row_id:row.into(),property_id:property.into(),
                value:LocalValue::Checkbox(next)}
        };
        self.apply_change(core,cmd)
    }
    pub fn poll(&mut self)->bool{
        let mut changed=false;
        while let Ok(reply)=self.rx.try_recv(){
            if !self.view.busy || reply.generation!=self.generation
                || self.view.source.as_deref()!=Some(&reply.source)
                || self.view.target.as_deref()!=Some(reply.target.as_path()) {continue;}
            let saving=self.view.stage==Stage::Saving;
            self.view.busy=false;
            match reply.result {
                Ok(Payload::Catalog(choices))=>{
                    self.view.choices=choices;
                    self.view.stage=Stage::Ready;
                    self.view.notice=Some("Выберите импортированную базу и ревизию снимка");
                }
                Ok(Payload::Loaded(base,hash)) if
                    self.view.base_id.as_deref()==Some(&base.base_id)
                    && self.view.digest.as_deref()==Some(&base.source_digest) =>
                {
                    self.effective=Some(base); self.disk_hash=hash;
                    self.view.stage=Stage::Ready;
                    if saving { self.view.draft=None; self.view.schema_draft=None;
                        self.view.selected=None; }
                    self.view.notice=if saving{Some("Правка сохранена локально через File Gate CAS")}
                        else{None};
                }
                _=>{
                    self.effective=None; self.disk_hash=None;
                    self.view.selected=None;
                    self.view.stage=Stage::NeedsReview;
                    if saving {
                        if let Some(draft)=self.view.draft.as_mut(){draft.frozen=true;}
                        if let Some(draft)=self.view.schema_draft.as_mut(){draft.frozen=true;}
                    }
                    self.view.notice=Some("Проверка данных или CAS не пройдена. Перечитайте локальную базу.");
                }
            }
            changed=true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn checkbox_and_snapshot_labels_are_explicit() {
        assert_eq!(checkbox(&EffectiveValue::Source(json!({"type":"checkbox","checkbox":true}))),Some(true));
        assert_eq!(checkbox(&EffectiveValue::EmptyLocal),Some(false));
        assert_eq!(checkbox(&EffectiveValue::Source(json!({"type":"formula","formula":{"number":7}}))),None);
        assert!(label(&EffectiveValue::Source(json!({"type":"formula","formula":{"number":7}}))).contains("снимок"));
    }
    #[test]
    fn missing_independent_source_never_creates_one() {
        let tmp=tempfile::tempdir().unwrap();
        let id="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        assert!(catalog(tmp.path(),id).is_err());
        assert!(!tmp.path().join("Notion-import").exists());
    }
}
