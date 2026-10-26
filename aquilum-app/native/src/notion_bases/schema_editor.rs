//! QA-only Akula-owned column create/rename drafts.
//! Column identity comes from a fresh UUID; display name is never an ID.
//! All changes go through the existing Core File Gate CAS, not Notion API.
use super::{BasesSession, Stage};
use aquilum_core::{
    Core, notion_api_probe::bases_edits::{Change, LocalKind},
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaMode {
    Create { local_id: String },
    Rename { property_id: String, original: String },
}
#[derive(Clone, Debug)]
pub struct SchemaDraftView {
    pub mode: SchemaMode,
    pub kind: LocalKind,
    pub name: String,
    pub frozen: bool,
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128
        && !name.chars().any(char::is_control)
}
fn kind_label(kind: LocalKind) -> &'static str {
    match kind {
        LocalKind::Text => "Текст",
        LocalKind::Number => "Число",
        LocalKind::Checkbox => "Флажок",
    }
}
fn source_kind(kind: &str) -> Option<LocalKind> {
    match kind {
        "rich_text" => Some(LocalKind::Text),
        "number" => Some(LocalKind::Number),
        "checkbox" => Some(LocalKind::Checkbox),
        _ => None,
    }
}
impl SchemaDraftView {
    pub fn kind_label(&self) -> &'static str { kind_label(self.kind) }
    pub fn is_new(&self) -> bool { matches!(self.mode, SchemaMode::Create{..}) }
}
impl BasesSession {
    fn can_begin_schema(&self) -> bool {
        self.view.stage==Stage::Ready && !self.view.busy
            && self.view.draft.is_none() && self.view.schema_draft.is_none()
            && self.effective.is_some()
    }
    pub fn begin_new_column(&mut self, kind: LocalKind) -> Result<(),()> {
        if !self.can_begin_schema() { return Err(()); }
        self.view.schema_draft=Some(SchemaDraftView {
            mode:SchemaMode::Create {local_id:uuid::Uuid::new_v4().to_string()},
            kind,name:String::new(),frozen:false,
        });
        self.view.notice=None;
        Ok(())
    }
    pub fn begin_rename_column(&mut self, property: &str) -> Result<(),()> {
        if !self.can_begin_schema() { return Err(()); }
        let base=self.effective.as_ref().ok_or(())?;
        let col=base.columns.iter().find(|c|c.property_id==property && c.local_only)
            .ok_or(())?;
        let kind=source_kind(&col.kind).ok_or(())?;
        self.view.schema_draft=Some(SchemaDraftView {
            mode:SchemaMode::Rename {
                property_id:col.property_id.clone(),original:col.name.clone(),
            },
            name:col.name.clone(),kind,frozen:false,
        });
        self.view.notice=None;
        Ok(())
    }
    pub fn update_schema_name(&mut self, name: &str) -> Result<(),()> {
        let Some(draft)=self.view.schema_draft.as_mut() else{return Err(())};
        if self.view.busy || self.view.stage!=Stage::Ready || draft.frozen {return Err(());}
        // Incomplete input is legal in RAM, but never in File Gate.
        if name.len()>128 || name.chars().any(char::is_control) {
            self.view.notice=Some("Имя колонки не должно превышать 128 байт");
            return Err(());
        }
        draft.name=name.to_owned();
        self.view.notice=None;
        Ok(())
    }
    pub fn cancel_schema(&mut self) -> bool {
        if self.view.busy || self.view.schema_draft.is_none() {return false;}
        self.view.schema_draft=None;
        self.view.notice=None;
        true
    }
    pub fn commit_schema(&mut self, core: Arc<Core>) -> Result<(),()> {
        if self.view.busy || self.view.stage!=Stage::Ready {return Err(());}
        let draft=self.view.schema_draft.as_ref().ok_or(())?;
        if draft.frozen {return Err(());}
        if !valid_name(&draft.name) {
            self.view.notice=Some("Имя колонки пустое, некорректное или слишком длинное");
            return Err(());
        }
        let change=match &draft.mode {
            SchemaMode::Create {local_id} => Change::AddColumn {
                base_id:self.view.base_id.clone().ok_or(())?,
                local_id:local_id.clone(),name:draft.name.clone(),kind:draft.kind,
            },
            SchemaMode::Rename {property_id,original} => {
                if *original==draft.name {
                    self.view.notice=Some("Имя не изменилось: запись не требуется");
                    return Err(());
                }
                Change::RenameLocalColumn {
                    property_id:property_id.clone(),name:draft.name.clone(),
                }
            }
        };
        self.apply_change(core,change)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::notion_api_probe::bases_edits::{
        EffectiveBase,EffectiveColumn,
    };
    fn session() -> BasesSession {
        let mut session=BasesSession::new(Arc::new(||{}));
        session.view.stage=Stage::Ready;
        session.view.base_id=Some("base".into());
        session.effective=Some(EffectiveBase {
            base_id:"base".into(), notion_source_id:"notion-source".into(),
            source_root:"root".into(),source_digest:"a".repeat(64),
            local_revision:0,saved_source_views:0,native_views_supported:false,
            columns:vec![
                EffectiveColumn {
                    property_id:"source-name".into(),name:"Name".into(),
                    kind:"rich_text".into(),local_only:false,editable_locally:true,
                },
                EffectiveColumn {
                    property_id:"local-title".into(),name:"Local Name".into(),
                    kind:"rich_text".into(),local_only:true,editable_locally:true,
                },
            ],rows:Vec::new(),
        });
        session
    }
    #[test]
    fn new_column_uses_distinct_stable_local_ids_and_does_not_write_on_typing() {
        let mut s=session();
        assert!(s.begin_new_column(LocalKind::Number).is_ok());
        let first=s.view().schema_draft.unwrap();
        assert!(uuid::Uuid::parse_str(match &first.mode {
            SchemaMode::Create {local_id} => local_id, _=>unreachable!(),
        }).is_ok());
        assert!(s.begin_new_column(LocalKind::Text).is_err());
        s.update_schema_name("Расходы").unwrap();
        assert_eq!(s.view().schema_draft.as_ref().unwrap().name,"Расходы");
        assert!(s.page(0,0)==false);
        assert!(s.reload().is_err());
        assert!(s.cancel_schema());
        s.begin_new_column(LocalKind::Number).unwrap();
        let second=s.view().schema_draft.unwrap();
        assert_ne!(first.mode,second.mode);
    }
    #[test]
    fn rename_preserves_property_id_and_refuses_source_column() {
        let mut s=session();
        assert!(s.begin_rename_column("source-name").is_err());
        s.begin_rename_column("local-title").unwrap();
        let draft=s.view().schema_draft.unwrap();
        assert_eq!(draft.kind,LocalKind::Text);
        assert!(matches!(draft.mode,SchemaMode::Rename {property_id,original}
            if property_id=="local-title" && original=="Local Name"));
    }
    #[test]
    fn invalid_names_cannot_be_committed() {
        for name in ["", "  ", "bad\ncolumn"] {
            assert!(!valid_name(name));
        }
        assert!(!valid_name(&"x".repeat(129)));
        assert!(valid_name("Название — v2"));
    }
}
