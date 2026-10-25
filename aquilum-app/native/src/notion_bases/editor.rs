//! Local plain-text/number cell drafts for the QA Bases panel.
//! A draft is never written on typing, Enter, page navigation or popup close.
use super::{BasesSession, Stage};
use aquilum_core::{
    Core, notion_api_probe::bases_edits::{Change, EffectiveValue, LocalValue},
};
use std::sync::Arc;

const MAX_TEXT_BYTES: usize = 32 * 1024;
const MAX_NUMBER_INPUT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind { Text, Number }
#[derive(Clone, Debug)]
pub struct DraftView {
    pub row_id: String,
    pub property_id: String,
    pub column_name: String,
    pub kind: EditKind,
    pub value: String,
    /// A rejected CAS operation must never become an automatic retry.
    /// Keep the user's text visible, but require cancel + fresh selection.
    pub frozen: bool,
}
fn editable_kind(name: &str) -> Option<EditKind> {
    match name {
        "title" | "rich_text" | "url" | "email" | "phone_number" =>
            Some(EditKind::Text),
        "number" => Some(EditKind::Number),
        _ => None,
    }
}
fn prefill(value: &EffectiveValue, kind: &str) -> Result<String, ()> {
    match value {
        EffectiveValue::Local(LocalValue::Text(s))
            if editable_kind(kind)==Some(EditKind::Text) => Ok(s.clone()),
        EffectiveValue::Local(LocalValue::Number(n)) if kind=="number" =>
            Ok(n.to_string()),
        EffectiveValue::EmptyLocal => Ok(String::new()),
        EffectiveValue::Source(raw) if raw.get("type").and_then(|v|v.as_str())==Some(kind) =>
            match kind {
                "title" | "rich_text" => raw.get(kind).and_then(|v|v.as_array())
                    .ok_or(())?.iter().map(|item|
                        item.get("plain_text").and_then(|v|v.as_str())
                            .ok_or(())).collect::<Result<String,()>>(),
                "number" => raw.get("number").filter(|v|!v.is_null())
                    .map_or(Ok(String::new()),|v|v.as_f64()
                        .map(|n|n.to_string()).ok_or(())),
                "url" | "email" | "phone_number" => raw.get(kind)
                    .map_or(Err(()),|v|if v.is_null(){Ok(String::new())}
                        else{v.as_str().map(str::to_owned).ok_or(())}),
                _ => Err(()),
            },
        _ => Err(()),
    }
}
fn parse(kind: EditKind, s: &str) -> Result<LocalValue, &'static str> {
    match kind {
        EditKind::Text if s.len()<=MAX_TEXT_BYTES => Ok(LocalValue::Text(s.to_owned())),
        EditKind::Text => Err("Локальный текст превышает 32 КиБ"),
        EditKind::Number => {
            let trimmed=s.trim();
            if trimmed.is_empty() || trimmed.len()>MAX_NUMBER_INPUT {
                return Err("Введите число, используя точку как разделитель");
            }
            let n=trimmed.parse::<f64>()
                .map_err(|_|"Некорректное число; используйте точку")?;
            if !n.is_finite() { return Err("NaN и бесконечность запрещены"); }
            Ok(LocalValue::Number(n))
        }
    }
}
impl BasesSession {
    pub fn begin_draft(&mut self, row: &str, property: &str)->Result<(),()> {
        if self.view.stage!=Stage::Ready || self.view.busy
            || self.view.draft.is_some() { return Err(()); }
        let base=self.effective.as_ref().ok_or(())?;
        let col=base.columns.iter().find(|c|c.property_id==property
            && c.editable_locally).ok_or(())?;
        let kind=editable_kind(&col.kind).ok_or(())?;
        let record=base.rows.iter().find(|r|r.akula_id==row).ok_or(())?;
        let cell=record.cells.iter().find(|c|c.property_id==property).ok_or(())?;
        let value=prefill(&cell.value,&col.kind)?;
        self.view.draft=Some(DraftView {
            row_id:row.to_owned(),property_id:property.to_owned(),
            column_name:col.name.clone(),kind,value,frozen:false,
        });
        self.view.notice=None;
        Ok(())
    }
    pub fn update_draft(&mut self, text: &str)->Result<(),()> {
        let Some(draft)=self.view.draft.as_mut() else { return Err(()); };
        if self.view.busy || self.view.stage!=Stage::Ready || draft.frozen {return Err(());}
        let limit=if draft.kind==EditKind::Text{MAX_TEXT_BYTES}else{MAX_NUMBER_INPUT};
        if text.len()>limit {
            self.view.notice=Some("Поле слишком длинное: значение не принято");
            return Err(());
        }
        draft.value=text.to_owned();
        self.view.notice=None;
        Ok(())
    }
    pub fn reject_draft_input(&mut self) {
        self.view.notice=Some("Не удалось прочитать поле: сохранение отменено");
    }
    pub fn cancel_draft(&mut self)->bool{
        if self.view.busy || self.view.draft.is_none(){return false;}
        self.view.draft=None;
        self.view.notice=None;
        true
    }
    pub fn commit_draft(&mut self, core:Arc<Core>)->Result<(),()> {
        if self.view.busy || self.view.stage!=Stage::Ready{return Err(());}
        let draft=self.view.draft.as_ref().ok_or(())?;
        if draft.frozen {return Err(());}
        let value=match parse(draft.kind,&draft.value) {
            Ok(v)=>v,
            Err(reason)=>{self.view.notice=Some(reason);return Err(());}
        };
        let change=Change::Set{row_id:draft.row_id.clone(),
            property_id:draft.property_id.clone(),value};
        self.apply_change(core,change)
    }
    pub(super) fn apply_change(&mut self, core:Arc<Core>,cmd:Change)->Result<(),()> {
        let target=self.view.target.clone().ok_or(())?;
        let source=self.view.source.clone().ok_or(())?;
        let digest=self.view.digest.clone().ok_or(())?;
        let base=self.view.base_id.clone().ok_or(())?;
        let hash=self.disk_hash.clone();
        self.start(Stage::Saving,move || {
            super::bases_edits::apply_approved(
                &core,&target,&source,&digest,hash.as_deref(),&[cmd],true
            ).map_err(|_| ())?;
            super::load(&target,&source,&digest,&base)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn text_draft_is_full_length_and_not_a_truncated_preview() {
        let original="Длинное значение ".repeat(20);
        let input=EffectiveValue::Source(json!({
            "type":"rich_text","rich_text":[{"plain_text":original}]
        }));
        assert_eq!(prefill(&input,"rich_text").unwrap(),original);
        assert_eq!(parse(EditKind::Text,&original).unwrap(),
            LocalValue::Text(original));
    }
    #[test]
    fn numeric_draft_rejects_nonfinite_ambiguous_and_blank_inputs() {
        for invalid in ["", " ", "NaN", "inf", "-inf","1,5","1 234","1e500"] {
            assert!(parse(EditKind::Number,invalid).is_err(),"{invalid}");
        }
        assert_eq!(parse(EditKind::Number," -1.25 ").unwrap(),
            LocalValue::Number(-1.25));
        assert!(parse(EditKind::Text,&"a".repeat(MAX_TEXT_BYTES+1)).is_err());
    }
    #[test]
    fn formulas_and_malformed_source_cannot_be_interpreted_as_plain_text() {
        assert!(editable_kind("formula").is_none());
        assert!(prefill(&EffectiveValue::Source(json!({"type":"rich_text",
            "rich_text":[{"href":"x"}]})),"rich_text").is_err());
        assert!(prefill(&EffectiveValue::Source(json!({
            "type":"formula","formula":{"string":"lookalike"}
        })),"rich_text").is_err());
    }
}
