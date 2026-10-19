//! AKAI-AUTH-00: test-only authority inventory, **not a permission gate**.
//!
//! A broker grant MUST NOT be inferred from this table: no HTTP principal,
//! host-issued vault ID, or consent exists in this phase. The inventory makes
//! new tools fail CI until their access and side-effect shape is reviewed.
//! Runtime enforcement belongs to AKAI-AUTH-01 with the existing plugin
//! PermissionBroker capability vocabulary and host-bound resource identities.

use crate::plugins::Capability;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Effect {
    HelpOnly,
    Read,
    Write,
    Destructive,
    UserInterface,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    None,
    Note,
    NoteSet,
    Vault,
    SearchResults,
    VaultList,
    CrossVault,
    UserInterface,
}

struct Contract {
    name: &'static str,
    effect: Effect,
    scope: Scope,
    required: &'static [Capability],
    approval: bool,
}

macro_rules! contract {
    ($name:literal, $effect:ident, $scope:ident, [$($capability:ident),*], $approval:literal) => {
        Contract {
            name: $name,
            effect: Effect::$effect,
            scope: Scope::$scope,
            required: &[$(Capability::$capability),*],
            approval: $approval,
        }
    };
}

// Conservative minimum capabilities. Some shapes (search hits, batches and
// cross-vault moves) demand multiple independent resource decisions. These
// cannot be enforced by validating only the string 'workspace' in JSON.
//
// This table deliberately uses the canonical plugin Capability enum, not
// a second string-based list of permissions for agent clients.
const CONTRACTS: &[Contract] = &[
    // Notes / files
    contract!("read_note", Read, NoteSet, [NotesRead], false),
    contract!("create_note", Write, NoteSet, [NotesCreate], false),
    contract!("update_note", Write, Note, [NotesPatch], false),
    contract!("rename_note", Write, Note, [NotesPatch], false),
    contract!("move_note", Write, CrossVault, [NotesPatch, NotesCreate, NotesTrash], false),
    contract!("delete_note", Destructive, Note, [NotesTrash], true),
    contract!("list_notes", Read, SearchResults, [NotesSearch], false),
    contract!("note_outline", Read, Note, [NotesRead], false),
    contract!("list_folders", Read, Vault, [NotesSearch], false),
    // Search / backlinks / knowledge derived from source notes
    contract!("search_notes", Read, SearchResults, [NotesSearch], false),
    contract!("dataview_syntax", HelpOnly, None, [], false),
    contract!("related_notes", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("linked_notes", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("backlinks", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("outgoing_links", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("wiki_sources", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("get_index_status", Read, Vault, [NotesSearch], false),
    // Metadata / trash (results, names and historical bytes also need grants)
    contract!("set_frontmatter", Write, Note, [NotesPatch], false),
    contract!("find_notes", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("list_trash", Read, SearchResults, [NotesRead, NotesSearch], false),
    contract!("restore_note", Write, Note, [NotesTrash, NotesCreate], false),
    contract!("empty_trash", Destructive, Vault, [NotesTrash], true),
    contract!("note_history", Read, Note, [NotesRead], false),
    contract!("read_version", Read, Note, [NotesRead], false),
    contract!("restore_version", Write, Note, [NotesPatch], false),
    contract!("name_version", Write, Note, [NotesPatch], false),
    // Workspace names, paths and UI state must not leak across agent scopes
    contract!("get_workspace", Read, Vault, [NotesRead], false),
    contract!("list_workspaces", Read, VaultList, [NotesRead], false),
    contract!("switch_workspace", UserInterface, UserInterface, [UiCommand, NotesRead], true),
    contract!("open_note", UserInterface, UserInterface, [UiCommand, NotesRead], true),
];

#[test]
fn every_mcp_tool_has_one_explicit_authority_contract() {
    let defined = super::definitions();
    let mut actual = BTreeSet::new();
    for tool in &defined {
        let name = tool["name"].as_str().expect("tool definition must have a name");
        assert!(actual.insert(name), "duplicated advertised MCP tool {name}");
    }
    let mut inventoried = BTreeSet::new();
    for item in CONTRACTS {
        assert!(inventoried.insert(item.name), "duplicated authority contract {}", item.name);
    }
    assert_eq!(defined.len(), 30, "review authority before adding a new MCP tool");
    assert_eq!(actual, inventoried, "unclassified MCP tools must fail CI");
}

#[test]
fn all_data_bearing_and_side_effecting_contracts_need_capabilities() {
    for item in CONTRACTS {
        if item.effect == Effect::HelpOnly {
            assert_eq!(item.scope, Scope::None);
            assert!(item.required.is_empty());
        } else {
            assert!(!item.required.is_empty(), "{} missing capability", item.name);
            assert_ne!(item.scope, Scope::None, "{} missing resource boundary", item.name);
        }
        assert!(
            !item.required.contains(&Capability::AiInvoke)
                && !item.required.contains(&Capability::NetworkFetch),
            "{} cannot turn note access into LLM or network authority",
            item.name
        );
        if matches!(item.effect, Effect::Destructive | Effect::UserInterface) {
            assert!(item.approval, "{} requires explicit user command/approval", item.name);
        }
    }
}

#[test]
fn metadata_and_bulk_operations_must_not_be_treated_as_a_single_note() {
    let by_name = |name| CONTRACTS.iter().find(|item| item.name == name).unwrap();
    for name in ["read_note", "create_note"] {
        assert_eq!(by_name(name).scope, Scope::NoteSet);
    }
    for name in [
        "search_notes", "find_notes", "list_notes", "list_trash",
        "backlinks", "wiki_sources", "related_notes", "linked_notes",
    ] {
        assert_eq!(by_name(name).scope, Scope::SearchResults);
    }
    assert_eq!(by_name("list_workspaces").scope, Scope::VaultList);
    assert_eq!(by_name("move_note").scope, Scope::CrossVault);
    assert_eq!(by_name("empty_trash").effect, Effect::Destructive);
}
