use super::{
    BrokerError, Capability, GrantScope, HostContext, NoteId, NoteResource,
    PermissionBroker, PluginManifest, Resource, VaultId,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::tempdir;

const VALID: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/valid-v1.json"
));

fn manifest(id: &str, capabilities: &[&str], domains: &[&str]) -> PluginManifest {
    let mut value: Value = serde_json::from_str(VALID).unwrap();
    value["id"] = json!(id);
    value["permissions"]["required"] = json!(capabilities);
    value["permissions"]["optional"] = json!([]);
    value["network_domains"] = json!(domains);
    PluginManifest::parse_json(&value.to_string()).unwrap()
}

fn register(
    broker: &PermissionBroker,
    vault: VaultId,
    id: &str,
    digest: [u8; 32],
) -> super::PluginInstanceHandle {
    broker
        .register_host_package(
            &manifest(
                id,
                &["notes.read", "notes.search", "notes.patch", "network.fetch", "ui.command"],
                &["api.example.org"],
            ),
            digest,
            vault,
        )
        .unwrap()
}

fn note(vault: VaultId, id: NoteId, path: &str) -> Resource {
    Resource::Note(NoteResource::from_host_index(vault, id, path).unwrap())
}

fn open(path: &Path) -> PermissionBroker {
    PermissionBroker::open_persistent(path).unwrap()
}

#[test]
fn persistent_consent_restores_only_matching_vault_package_and_publisher() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("permissions.sqlite");
    let vault = VaultId::new_for_host();
    let other_vault = VaultId::new_for_host();
    let id = NoteId::new_for_host();
    let target = note(vault, id, "Projects/alpha.md");
    let ctx = HostContext::from_host_state(vault, Some(id));

    {
        let broker = open(&file);
        let a = register(&broker, vault, "org.example.word-count", [1; 32]);
        broker.grant_host_approved_persistent(
            a, Capability::NotesRead, GrantScope::folder("Projects").unwrap(), None,
        ).unwrap();
        broker.grant_host_approved(a, Capability::NotesSearch, GrantScope::WholeVault).unwrap();
        assert!(broker.authorize(a, ctx, Capability::NotesRead, &target).is_ok());
        assert!(broker.authorize(a, ctx, Capability::NotesSearch, &target).is_ok());
    }

    let broker = open(&file);
    let same = register(&broker, VaultId::from_uuid_for_host(vault.as_uuid()),
        "org.example.word-count", [1; 32]);
    assert!(broker.authorize(same, ctx, Capability::NotesRead, &target).is_ok());
    // Nonpersistent session-only search does not survive restart.
    assert_eq!(broker.authorize(same, ctx, Capability::NotesSearch, &target).unwrap_err(),
        BrokerError::Denied);

    let changed_digest = register(&broker, vault, "org.example.word-count", [2; 32]);
    let changed_plugin = register(&broker, vault, "org.example.other", [1; 32]);
    let foreign_vault = register(&broker, other_vault, "org.example.word-count", [1; 32]);
    for different in [changed_digest, changed_plugin] {
        assert_eq!(broker.authorize(different, ctx, Capability::NotesRead, &target).unwrap_err(),
            BrokerError::Denied);
    }
    assert_eq!(broker.authorize(foreign_vault,
        HostContext::from_host_state(other_vault, Some(id)),
        Capability::NotesRead, &note(other_vault,id,"Projects/alpha.md")).unwrap_err(),
        BrokerError::Denied);
}

#[test]
fn revoke_is_durable_and_in_flight_lease_cannot_survive_it() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("permissions.sqlite");
    let vault = VaultId::new_for_host();
    let active = NoteId::new_for_host();
    let ctx = HostContext::from_host_state(vault,Some(active));
    let resource = note(vault,active,"Tasks/todo.md");
    {
        let broker = open(&file);
        let a = register(&broker,vault,"org.example.word-count",[3;32]);
        broker.grant_host_approved_persistent(a,Capability::NotesRead,GrantScope::CurrentNote,None).unwrap();
        let lease = broker.authorize(a,ctx,Capability::NotesRead,&resource).unwrap();
        broker.revoke_all_for_host(a).unwrap();
        assert_eq!(broker.revalidate_lease(&lease,ctx).unwrap_err(),BrokerError::StaleLease);
        assert_eq!(broker.authorize(a,ctx,Capability::NotesRead,&resource).unwrap_err(),
            BrokerError::Denied);
    }
    let broker = open(&file);
    let a = register(&broker,vault,"org.example.word-count",[3;32]);
    assert_eq!(broker.authorize(a,ctx,Capability::NotesRead,&resource).unwrap_err(),
        BrokerError::Denied);
    let connection = Connection::open(&file).unwrap();
    let events: Vec<String> = connection
        .prepare("SELECT event FROM plugin_audit ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0)).unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(events, ["grant","revoke"]);
}

#[test]
fn concurrent_brokers_fail_closed_when_another_instance_revokes() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("permissions.sqlite");
    let vault = VaultId::new_for_host();
    let active = NoteId::new_for_host();
    let ctx = HostContext::from_host_state(vault,Some(active));
    let resource = note(vault,active,"notes/active.md");
    let first = open(&file);
    let a = register(&first,vault,"org.example.word-count",[4;32]);
    first.grant_host_approved_persistent(a,Capability::NotesRead,GrantScope::WholeVault,None).unwrap();
    let second = open(&file);
    let b = register(&second,vault,"org.example.word-count",[4;32]);
    let previous = second.authorize(b,ctx,Capability::NotesRead,&resource).unwrap();
    first.revoke_all_for_host(a).unwrap();
    assert_eq!(second.authorize(b,ctx,Capability::NotesRead,&resource).unwrap_err(),
        BrokerError::StaleLease);
    assert_eq!(second.revalidate_lease(&previous,ctx).unwrap_err(),BrokerError::StaleLease);
    // Unload/re-register against the current snapshot, which now has no grants.
    second.unregister_host_instance(b).unwrap();
    let c = register(&second,vault,"org.example.word-count",[4;32]);
    assert_eq!(second.authorize(c,ctx,Capability::NotesRead,&resource).unwrap_err(),
        BrokerError::Denied);
}

#[test]
fn declined_capabilities_and_path_scopes_remain_denied_after_restart() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("permissions.sqlite");
    let vault = VaultId::new_for_host();
    let note_id = NoteId::new_for_host();
    let scope = GrantScope::selected_notes([note_id]).unwrap();
    {
        let broker = open(&file);
        let a = register(&broker,vault,"org.example.word-count",[5;32]);
        broker.grant_host_approved_persistent(a,Capability::NotesRead,scope.clone(),None).unwrap();
        assert_eq!(broker.grant_host_approved_persistent(a,Capability::ClipboardRead,
            GrantScope::Action,None).unwrap_err(),BrokerError::UndeclaredCapability);
    }
    let broker = open(&file);
    let a = register(&broker,vault,"org.example.word-count",[5;32]);
    let ctx = HostContext::from_host_state(vault,None);
    assert!(broker.authorize(a,ctx,Capability::NotesRead,
        &note(vault,note_id,"Home/a.md")).is_ok());
    assert_eq!(broker.authorize(a,ctx,Capability::NotesRead,
        &note(vault,NoteId::new_for_host(),"Home/b.md")).unwrap_err(),BrokerError::Denied);
}

#[test]
fn expiry_is_enforced_during_load_and_authorization() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("permissions.sqlite");
    let vault = VaultId::new_for_host();
    let active = NoteId::new_for_host();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let broker = open(&file);
    let a = register(&broker,vault,"org.example.word-count",[6;32]);
    assert_eq!(broker.grant_host_approved_persistent(a,Capability::NotesRead,
        GrantScope::CurrentNote,Some(now-1)).unwrap_err(),BrokerError::InvalidExpiry);
    broker.grant_host_approved_persistent(a,Capability::NotesRead,GrantScope::CurrentNote,
        Some(now+3600)).unwrap();
    let ctx = HostContext::from_host_state(vault,Some(active));
    let resource=note(vault,active,"x.md");
    assert!(broker.authorize(a,ctx,Capability::NotesRead,&resource).is_ok());
    drop(broker);
    // Test expiration without sleeps or clocks: modify only the test database.
    let db=Connection::open(&file).unwrap();
    db.execute("UPDATE plugin_grants SET expires_at=1",[]).unwrap();
    drop(db);
    let broker=open(&file);
    let a=register(&broker,vault,"org.example.word-count",[6;32]);
    assert_eq!(broker.authorize(a,ctx,Capability::NotesRead,&resource).unwrap_err(),BrokerError::Denied);
}

#[test]
fn unsupported_schema_and_corrupt_grant_fail_closed() {
    let dir=tempdir().unwrap();
    let file=dir.path().join("permissions.sqlite");
    let vault=VaultId::new_for_host();
    {
        let broker=open(&file);
        let a=register(&broker,vault,"org.example.word-count",[7;32]);
        broker.grant_host_approved_persistent(a,Capability::NotesRead,
            GrantScope::folder("Projects").unwrap(),None).unwrap();
    }
    {
        let db=Connection::open(&file).unwrap();
        db.execute("UPDATE plugin_grants SET scope_value='../private'",[]).unwrap();
    }
    let broker=open(&file);
    assert_eq!(broker.register_host_package(
        &manifest("org.example.word-count",&["notes.read"],&[]),
        [7;32],vault).unwrap_err(),BrokerError::CorruptStore);
    drop(broker);
    {
        let db=Connection::open(&file).unwrap();
        db.execute_batch("PRAGMA user_version=99;").unwrap();
    }
    assert!(matches!(PermissionBroker::open_persistent(&file),Err(BrokerError::UnsupportedStore)));
}

#[test]
fn aborted_sqlite_write_does_not_issue_any_in_memory_grant() {
    let dir=tempdir().unwrap();
    let file=dir.path().join("permissions.sqlite");
    let broker=open(&file);
    let vault=VaultId::new_for_host();
    let a=register(&broker,vault,"org.example.word-count",[8;32]);
    let current=NoteId::new_for_host();
    let ctx=HostContext::from_host_state(vault,Some(current));
    let note=note(vault,current,"Doc.md");
    {
        let db=Connection::open(&file).unwrap();
        db.execute_batch("CREATE TRIGGER abort_new_grants BEFORE INSERT ON plugin_grants
                          BEGIN SELECT RAISE(ABORT,'test abort'); END;").unwrap();
    }
    assert_eq!(broker.grant_host_approved_persistent(a,Capability::NotesRead,
        GrantScope::CurrentNote,None).unwrap_err(),BrokerError::StoreFailure);
    assert_eq!(broker.authorize(a,ctx,Capability::NotesRead,&note).unwrap_err(),BrokerError::Denied);
    let db=Connection::open(&file).unwrap();
    let n: i64=db.query_row("SELECT COUNT(*) FROM plugin_grants",[],|r|r.get(0)).unwrap();
    assert_eq!(n,0);
}

#[test]
fn scope_and_domain_restrictions_survive_restart_without_secret_audit() {
    let dir=tempdir().unwrap();
    let file=dir.path().join("permissions.sqlite");
    let vault=VaultId::new_for_host();
    {
        let broker=open(&file);
        let a=register(&broker,vault,"org.example.word-count",[9;32]);
        broker.grant_host_approved_persistent(a,Capability::NotesRead,
            GrantScope::folder("SecretProject/Plans").unwrap(),None).unwrap();
        broker.grant_host_approved_persistent(a,Capability::NetworkFetch,
            GrantScope::network_host("api.example.org").unwrap(),None).unwrap();
    }
    let broker=open(&file);
    let a=register(&broker,vault,"org.example.word-count",[9;32]);
    let ctx=HostContext::from_host_state(vault,None);
    assert!(broker.authorize(a,ctx,Capability::NotesRead,
        &note(vault,NoteId::new_for_host(),"SecretProject/Plans/x.md")).is_ok());
    assert_eq!(broker.authorize(a,ctx,Capability::NotesRead,
        &note(vault,NoteId::new_for_host(),"SecretProject/Planning/x.md")).unwrap_err(),
        BrokerError::Denied);
    assert!(broker.authorize(a,ctx,Capability::NetworkFetch,
        &Resource::Network(super::NetworkHost::parse_declared_host("api.example.org").unwrap())).is_ok());
    assert_eq!(broker.authorize(a,ctx,Capability::NetworkFetch,
        &Resource::Network(super::NetworkHost::parse_declared_host("cdn.example.org").unwrap())).unwrap_err(),
        BrokerError::Denied);
    let db=Connection::open(&file).unwrap();
    let private_audit: i64=db.query_row("SELECT COUNT(*) FROM plugin_audit
      WHERE capability LIKE '%SecretProject%' OR scope_kind LIKE '%SecretProject%'",[],
        |r|r.get(0)).unwrap();
    assert_eq!(private_audit,0);
    let grant_total: i64=db.query_row("SELECT COUNT(*) FROM plugin_grants",[],|r|r.get(0)).unwrap();
    assert_eq!(grant_total,2);
}

#[test]
fn in_memory_broker_never_pretends_to_persist() {
    let broker=PermissionBroker::new();
    let vault=VaultId::new_for_host();
    let a=register(&broker,vault,"org.example.word-count",[10;32]);
    assert_eq!(broker.grant_host_approved_persistent(a,Capability::NotesRead,
        GrantScope::CurrentNote,None).unwrap_err(),BrokerError::StoreUnavailable);
}

#[test]
fn time_limited_scope_does_not_accept_undeclared_network_hosts() {
    let dir=tempdir().unwrap();
    let broker=open(&dir.path().join("permissions.sqlite"));
    let a=register(&broker,VaultId::new_for_host(),"org.example.word-count",[11;32]);
    assert_eq!(broker.grant_host_approved_persistent(a,Capability::NetworkFetch,
        GrantScope::network_host("evil.example.org").unwrap(),Some(i64::MAX)).unwrap_err(),
        BrokerError::UndeclaredCapability);
}
