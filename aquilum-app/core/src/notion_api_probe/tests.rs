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
        (ReadRequest::Markdown(A.into()), json!({"object":"page_markdown","id":A,"markdown":"hello","truncated":false,"unknown_block_ids":[],"warnings":[]})),
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
    assert_eq!(report.summary().link_counts.get("contains"), Some(&1));
    assert_eq!(report.summary().link_counts.get("mention"), Some(&1));
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
        (ReadRequest::Markdown(A.into()),json!({"object":"page_markdown","id":A,"markdown":"","truncated":false,"unknown_block_ids":[],"warnings":[]})),
        (ReadRequest::BlockChildren{id:A.into(),cursor:None},list(vec![],false,None)),
    ])};
    let report=inspect_root(&mut c,A,ProbeLimits::default()).unwrap();
    assert!(report.links.iter().any(|e|e.kind=="relation"&&e.to==B));
    assert_eq!(report.requests,4);
}
#[test]
fn unrecognized_partial_markdown_is_reported() {
    let mut c=simple();
    c.replies.get_mut(1).unwrap().1=json!({"object":"page_markdown","id":A,"markdown":"partial","truncated":true,"unknown_block_ids":[B],"warnings":[]});
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
        (ReadRequest::Markdown(A.into()),json!({"object":"page_markdown","id":A,"markdown":"","truncated":false,"unknown_block_ids":[],"warnings":[]})),
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


#[test]
fn relation_26_targets_cross_page_preserves_identity_without_claiming_completion() {
    const PROPERTY: &str = "Stable%3Aref";
    let targets: Vec<String> = (1..=26)
        .map(|n| format!("00000000-0000-0000-0000-{n:012}"))
        .collect();
    let first: Vec<Value> = targets[..25].iter()
        .map(|id| json!({"object":"property_item","type":"relation","relation":{"id":id}}))
        .collect();
    let second: Vec<Value> = targets[25..].iter()
        .map(|id| json!({"object":"property_item","type":"relation","relation":{"id":id}}))
        .collect();
    let mut client = Fake { replies: VecDeque::from([
        (ReadRequest::Page(A.into()),
            json!({"object":"page","id":A,"properties":{
                "Linked":{"type":"relation","id":PROPERTY}}})),
        (ReadRequest::PageProperty {page:A.into(),property:PROPERTY.into(),cursor:None},
            list(first,true,Some("page-2"))),
        (ReadRequest::PageProperty {page:A.into(),property:PROPERTY.into(),cursor:Some("page-2".into())},
            list(second,false,None)),
        (ReadRequest::Markdown(A.into()),
            json!({"object":"page_markdown","id":A,"markdown":"fixture","truncated":false,"unknown_block_ids":[],"warnings":[]})),
        (ReadRequest::BlockChildren {id:A.into(),cursor:None},list(vec![],false,None)),
    ]) };
    let report = inspect_root(&mut client,A,ProbeLimits::default()).unwrap();
    let relations: Vec<&Link> = report.links.iter().filter(|e| e.kind == "relation").collect();
    assert_eq!(relations.len(),26);
    assert_eq!(report.requests,5);
    assert_eq!(report.unresolved_target_ids.len(),26);
    assert!(!report.root_traversal_complete);
    assert!(!report.workspace_complete);
    assert!(relations.iter().all(|e| e.from == A && e.property_id.as_deref() == Some(PROPERTY)));
    assert!(targets.iter().all(|id| relations.iter().any(|e| &e.to == id)));
    assert!(client.replies.is_empty());
}


#[test]
fn two_linked_databases_with_multiple_views_preserve_schema_and_relation() {
    // Synthetic analogue of the isolated two-database Notion acceptance corpus.
    // No credentials or workspace-specific identifiers enter this test.
    const ROOT: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const DB_A: &str = "bbbbbbbb-0000-0000-0000-000000000001";
    const DB_B: &str = "bbbbbbbb-0000-0000-0000-000000000002";
    const DS_A: &str = "cccccccc-0000-0000-0000-000000000001";
    const DS_B: &str = "cccccccc-0000-0000-0000-000000000002";
    const VIEW_A: &str = "dddddddd-0000-0000-0000-000000000001";
    const VIEW_A2: &str = "dddddddd-0000-0000-0000-000000000002";
    const VIEW_B: &str = "dddddddd-0000-0000-0000-000000000003";
    const ROW_A: &str = "eeeeeeee-0000-0000-0000-000000000001";
    const ROW_B: &str = "eeeeeeee-0000-0000-0000-000000000002";
    const REL: &str = "test%3Arelation";
    let mut client = Fake { replies: VecDeque::from(vec![
        (ReadRequest::Page(ROOT.into()),
            json!({"object":"page","id":ROOT,"properties":{}})),
        (ReadRequest::Markdown(ROOT.into()),
            json!({"object":"page_markdown","id":ROOT,"markdown":"root","truncated":false,"unknown_block_ids":[],"warnings":[]})),
        (ReadRequest::BlockChildren {id:ROOT.into(),cursor:None},
            list(vec![
                json!({"object":"block","id":DB_A,"type":"child_database","has_children":false}),
                json!({"object":"block","id":DB_B,"type":"child_database","has_children":false}),
            ],false,None)),
        (ReadRequest::Database(DB_A.into()),
            json!({"object":"database","id":DB_A,"data_sources":[{"id":DS_A}]})),
        (ReadRequest::Views {id:DB_A.into(),cursor:None},
            list(vec![
                json!({"object":"view","id":VIEW_A}),
                json!({"object":"view","id":VIEW_A2}),
            ],false,None)),
        (ReadRequest::View(VIEW_A.into()),
            json!({"object":"view","id":VIEW_A,"type":"table"})),
        (ReadRequest::View(VIEW_A2.into()),
            json!({"object":"view","id":VIEW_A2,"type":"board"})),
        (ReadRequest::Database(DB_B.into()),
            json!({"object":"database","id":DB_B,"data_sources":[{"id":DS_B}]})),
        (ReadRequest::Views {id:DB_B.into(),cursor:None},
            list(vec![json!({"object":"view","id":VIEW_B})],false,None)),
        (ReadRequest::View(VIEW_B.into()),
            json!({"object":"view","id":VIEW_B,"type":"table"})),
        (ReadRequest::DataSource(DS_A.into()),
            json!({"object":"data_source","id":DS_A,"properties":{
                "Linked tests":{"id":REL,"type":"relation","relation":{"data_source_id":DS_B}}
            }})),
        (ReadRequest::DataSourceRows {id:DS_A.into(),cursor:None},
            list(vec![json!({"object":"page","id":ROW_A})],false,None)),
        (ReadRequest::DataSource(DS_B.into()),
            json!({"object":"data_source","id":DS_B,"properties":{
                "Status":{"id":"state","type":"select"}
            }})),
        (ReadRequest::DataSourceRows {id:DS_B.into(),cursor:None},
            list(vec![json!({"object":"page","id":ROW_B})],false,None)),
        (ReadRequest::Page(ROW_A.into()),
            json!({"object":"page","id":ROW_A,"properties":{
                "Linked tests":{"id":REL,"type":"relation","relation":[{"id":ROW_B}]}
            }})),
        (ReadRequest::PageProperty {page:ROW_A.into(),property:REL.into(),cursor:None},
            list(vec![json!({"object":"property_item","type":"relation","relation":{"id":ROW_B}})],false,None)),
        (ReadRequest::Markdown(ROW_A.into()),
            json!({"object":"page_markdown","id":ROW_A,"markdown":"row A","truncated":false,"unknown_block_ids":[],"warnings":[]})),
        (ReadRequest::Page(ROW_B.into()),
            json!({"object":"page","id":ROW_B,"properties":{"Status":{"type":"select"}}})),
        (ReadRequest::Markdown(ROW_B.into()),
            json!({"object":"page_markdown","id":ROW_B,"markdown":"row B","truncated":false,"unknown_block_ids":[],"warnings":[]})),
        (ReadRequest::BlockChildren {id:ROW_A.into(),cursor:None},list(vec![],false,None)),
        (ReadRequest::BlockChildren {id:ROW_B.into(),cursor:None},list(vec![],false,None)),
    ]) };
    let report = inspect_root(&mut client,ROOT,ProbeLimits::default()).unwrap();
    let counts = &report.summary().counts;
    assert_eq!(counts.get("database"),Some(&2));
    assert_eq!(counts.get("data_source"),Some(&2));
    assert_eq!(counts.get("view"),Some(&3));
    assert_eq!(counts.get("page"),Some(&3)); // root + two rows, not one per view
    assert_eq!(report.requests,21);
    assert!(report.root_traversal_complete);
    assert!(!report.workspace_complete);
    assert!(report.unresolved_target_ids.is_empty());
    let relations: Vec<_> = report.links.iter().filter(|edge| edge.kind=="relation").collect();
    assert_eq!(relations.len(),1);
    assert_eq!(relations[0].from,ROW_A);
    assert_eq!(relations[0].to,ROW_B);
    assert_eq!(relations[0].property_id.as_deref(),Some(REL));
    let source = report.objects.iter()
        .find(|o| o.kind=="data_source" && o.id==DS_A).unwrap();
    assert_eq!(source.json.pointer("/properties/Linked tests/relation/data_source_id")
        .and_then(Value::as_str),Some(DS_B));
    assert!(client.replies.is_empty());
}

#[test]
fn markdown_identity_mismatch_is_rejected() {
    let mut client = simple();
    client.replies.get_mut(1).unwrap().1["id"] = json!(B);
    assert!(inspect_root(&mut client,A,ProbeLimits::default())
        .unwrap_err().contains("markdown page ID mismatch"));
}

#[test]
fn markdown_warnings_block_complete_root_claim() {
    let mut client = simple();
    client.replies.get_mut(1).unwrap().1["warnings"] =
        json!([{"code":"unsupported_block","message":"content not represented"}]);
    let report = inspect_root(&mut client,A,ProbeLimits::default()).unwrap();
    assert!(!report.root_traversal_complete);
    assert!(report.warnings.iter().any(|x| x.contains("markdown parser warnings")));
}

#[test]
fn malformed_relation_property_cannot_disappear_from_the_graph() {
    let mut client = simple();
    client.replies.front_mut().unwrap().1["properties"] =
        json!({"Relation":{"type":"relation"}});
    assert!(inspect_root(&mut client,A,ProbeLimits::default())
        .unwrap_err().contains("relation property missing stable ID"));
}

#[test]
fn malformed_relation_target_fails_instead_of_being_skipped() {
    let mut client = Fake { replies: VecDeque::from([
        (ReadRequest::Page(A.into()),json!({"object":"page","id":A,
            "properties":{"Linked":{"type":"relation","id":"ref"}}})),
        (ReadRequest::PageProperty {page:A.into(),property:"ref".into(),cursor:None},
            list(vec![json!({"object":"property_item","type":"relation",
                "relation":{}})],false,None)),
    ]) };
    assert!(inspect_root(&mut client,A,ProbeLimits::default())
        .unwrap_err().contains("relation item missing target ID"));
}

#[test]
fn missing_optional_markdown_warnings_is_valid() {
    let mut client = simple();
    client.replies.get_mut(1).unwrap().1.as_object_mut().unwrap().remove("warnings");
    let report = inspect_root(&mut client,A,ProbeLimits::default()).unwrap();
    assert!(report.root_traversal_complete);
}

#[test]
fn malformed_markdown_warnings_field_is_rejected() {
    let mut client = simple();
    client.replies.get_mut(1).unwrap().1["warnings"] = json!("not an array");
    assert!(inspect_root(&mut client,A,ProbeLimits::default())
        .unwrap_err().contains("warnings must be an array"));
}

#[test]
fn markdown_without_optional_warnings_is_accepted() {
    let mut client = simple();
    client.replies.get_mut(1).unwrap().1.as_object_mut().unwrap().remove("warnings");
    let report = inspect_root(&mut client,A,ProbeLimits::default()).unwrap();
    assert!(report.root_traversal_complete);
    assert!(report.warnings.is_empty());
}

#[test]
fn malformed_optional_markdown_warnings_are_rejected() {
    let mut client = simple();
    client.replies.get_mut(1).unwrap().1["warnings"] = json!("not an array");
    assert!(inspect_root(&mut client,A,ProbeLimits::default())
        .unwrap_err().contains("warnings must be an array"));
}
