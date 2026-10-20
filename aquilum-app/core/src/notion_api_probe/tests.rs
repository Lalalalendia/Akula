use super::*;
use serde_json::json;
use std::collections::VecDeque;
const A: &str = "11111111-1111-1111-1111-111111111111";
const B: &str = "22222222-2222-2222-2222-222222222222";
struct Fake { replies: VecDeque<(ReadRequest, Value)> }
impl NotionReadTransport for Fake {
    fn read(&mut self, req: &ReadRequest) -> Result<Value, String> {
        let (expected, value) = self.replies.pop_front().ok_or("unexpected API request")?;
        if &expected != req { return Err(format!("wrong API request: {req:?}, expected {expected:?}")); }
        Ok(value)
    }
}
fn list(results: Vec<Value>, more: bool, cursor: Option<&str>) -> Value {
    json!({"object":"list","results":results,"has_more":more,"next_cursor":cursor})
}
fn simple() -> Fake {
    Fake { replies: VecDeque::from([
        (ReadRequest::Page(A.into()),json!({"object":"page","id":A,"properties":{}})),
        (ReadRequest::Markdown(A.into()), json!({"markdown":"hello","truncated":false,"unknown_block_ids":[]})),
        (ReadRequest::BlockChildren {id:A.into(),cursor:None},list(vec![
            json!({"object":"block","id":B,"type":"paragraph","has_children":false,
                "paragraph":{"rich_text":[{"type":"mention","mention":{"type":"page","page":{"id":A}}}]}})
        ],false,None)),
    ]) }
}
#[test]
fn full_root_inventory_with_mention_and_no_writes() {
    let mut client = simple();
    let report = inspect_root(&mut client,A,ProbeLimits::default()).unwrap();
    assert!(report.root_traversal_complete);
    assert!(!report.workspace_complete);
    assert_eq!(report.requests,3);
    assert_eq!(report.objects.len(),3);
    assert!(report.links.iter().any(|e|e.kind=="mention" && e.to==A));
    assert!(report.links.iter().any(|e|e.kind=="contains" && e.to==B));
    assert_eq!(report.summary().counts["block"],1);
    assert!(client.replies.is_empty());
}
#[test]
fn incomplete_list_is_refused() {
    let mut c=simple();
    c.replies.back_mut().unwrap().1 = list(vec![],true,None);
    assert!(inspect_root(&mut c,A,ProbeLimits::default()).unwrap_err().contains("cursor"));
}
#[test]
fn request_budget_fails_without_returning_partial_as_success() {
    let mut c=simple();
    assert!(inspect_root(&mut c,A,ProbeLimits{max_requests:2,..ProbeLimits::default()}).is_err());
}
#[test]
fn repeated_cursor_is_detected() {
    let mut c=simple();
    c.replies.back_mut().unwrap().1=list(vec![],true,Some("repeat"));
    c.replies.push_back((ReadRequest::BlockChildren {id:A.into(),cursor:Some("repeat".into())},list(vec![],true,Some("repeat"))));
    assert!(inspect_root(&mut c,A,ProbeLimits::default()).unwrap_err().contains("repeating cursor"));
}
#[test]
fn relation_property_fetches_first_page_once() {
    let prop="abc";
    let mut c=Fake {replies:VecDeque::from([
        (ReadRequest::Page(A.into()),json!({"object":"page","id":A,"properties":{"Rel":{"type":"relation","id":prop}}})),
        (ReadRequest::PageProperty{page:A.into(),property:prop.into(),cursor:None},
            list(vec![json!({"object":"property_item","type":"relation","relation":{"id":B}})],false,None)),
        (ReadRequest::Markdown(A.into()),json!({"markdown":"","truncated":false,"unknown_block_ids":[]})),
        (ReadRequest::BlockChildren{id:A.into(),cursor:None},list(vec![],false,None)),
    ])};
    let report=inspect_root(&mut c,A,ProbeLimits::default()).unwrap();
    assert!(report.links.iter().any(|e|e.kind=="relation"&&e.to==B));
    assert_eq!(report.requests,4);
}
#[test]
fn unrecognized_partial_markdown_is_reported() {
    let mut c=simple();
    c.replies.get_mut(1).unwrap().1=json!({"markdown":"partial","truncated":true,"unknown_block_ids":[B]});
    let report=inspect_root(&mut c,A,ProbeLimits::default()).unwrap();
    assert_eq!(report.warnings.len(),2);
    assert!(!report.root_traversal_complete);
    assert!(!report.workspace_complete);
}

#[test]
fn relation_edge_preserves_stable_property_id() {
    let mut c=Fake {replies:VecDeque::from([
        (ReadRequest::Page(A.into()),json!({"object":"page","id":A,"properties":{"Связь":{"type":"relation","id":"X%3Aa"}}})),
        (ReadRequest::PageProperty{page:A.into(),property:"X%3Aa".into(),cursor:None},
            list(vec![json!({"object":"property_item","type":"relation","relation":{"id":B}})],false,None)),
        (ReadRequest::Markdown(A.into()),json!({"markdown":"","truncated":false,"unknown_block_ids":[]})),
        (ReadRequest::BlockChildren{id:A.into(),cursor:None},list(vec![],false,None)),
    ])};
    let report=inspect_root(&mut c,A,ProbeLimits::default()).unwrap();
    assert_eq!(report.links.iter().find(|e|e.kind=="relation").unwrap().property_id.as_deref(),Some("X%3Aa"));
    assert!(!report.root_traversal_complete); // target external to requested root
}
#[test]
fn synced_block_origin_kept_as_typed_edge() {
    let mut c=simple();
    c.replies.back_mut().unwrap().1=list(vec![
        json!({"object":"block","id":B,"type":"synced_block","has_children":false,
            "synced_block":{"synced_from":{"block_id":A}}})
    ],false,None);
    let report=inspect_root(&mut c,A,ProbeLimits::default()).unwrap();
    assert!(report.links.iter().any(|e|e.kind=="synced_from"&&e.from==B&&e.to==A));
}
#[test]
fn mentions_in_page_properties_are_detected() {
    let mut c=simple();
    c.replies.front_mut().unwrap().1=json!({"object":"page","id":A,
        "properties":{"Note":{"type":"rich_text","rich_text":[{"type":"mention",
        "mention":{"type":"page","page":{"id":B}}}]}}});
    let report=inspect_root(&mut c,A,ProbeLimits::default()).unwrap();
    assert!(report.links.iter().any(|e|e.kind=="mention"&&e.from==A&&e.to==B));
}
