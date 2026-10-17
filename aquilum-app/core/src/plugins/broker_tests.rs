use super::{
    BrokerError, Capability, GrantScope, HostContext, NetworkHost, NoteId, NoteResource,
    PermissionBroker, PluginManifest, RelativeVaultPath, Resource, VaultId,
};
use serde_json::{Value, json};

const VALID: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/valid-v1.json"
));

fn manifest(requested: &[&str], domains: &[&str]) -> PluginManifest {
    let mut v: Value = serde_json::from_str(VALID).unwrap();
    v["permissions"]["required"] = json!(requested);
    v["permissions"]["optional"] = json!([]);
    v["network_domains"] = json!(domains);
    PluginManifest::parse_json(&v.to_string()).unwrap()
}

fn fixture() -> (PermissionBroker, super::PluginInstanceHandle, VaultId, NoteId) {
    let broker = PermissionBroker::new();
    let vault = VaultId::new_for_host();
    let active = NoteId::new_for_host();
    let m = manifest(
        &[
            "ui.command", "notes.read", "notes.search", "notes.patch",
            "notes.create", "events.subscribe", "network.fetch",
        ],
        &["api.example.org", "cdn.example.org"],
    );
    let instance = broker.register_host_package(&m, [0xAA; 32], vault).unwrap();
    (broker, instance, vault, active)
}

fn note(vault: VaultId, id: NoteId, path: &str) -> Resource {
    Resource::Note(NoteResource::from_host_index(vault, id, path).unwrap())
}

#[test]
fn manifest_requests_never_issue_grants_and_unknown_identity_is_denied() {
    let (broker, handle, vault, active) = fixture();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let file = note(vault, active, "Tasks/today.md");
    for action in [
        Capability::NotesRead,
        Capability::NotesSearch,
        Capability::NotesPatch,
        Capability::EventsSubscribe,
    ] {
        assert_eq!(broker.authorize(handle, ctx, action, &file).unwrap_err(), BrokerError::Denied);
    }
    assert_eq!(
        broker.authorize(handle, ctx, Capability::UiCommand, &Resource::Ui).unwrap_err(),
        BrokerError::Denied
    );
    let different = super::PluginInstanceHandle(uuid::Uuid::new_v4());
    assert_eq!(
        broker.authorize(different, ctx, Capability::NotesRead, &file).unwrap_err(),
        BrokerError::UnknownInstance
    );
}

#[test]
fn current_note_follows_only_host_owned_active_note_and_vault() {
    let (broker, handle, vault, active) = fixture();
    let another = NoteId::new_for_host();
    let foreign = VaultId::new_for_host();
    broker.grant_host_approved(handle, Capability::NotesRead, GrantScope::CurrentNote).unwrap();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let allowed = note(vault, active, "Plan/day.md");
    let other = note(vault, another, "Plan/other.md");
    let lease = broker.authorize(handle, ctx, Capability::NotesRead, &allowed).unwrap();
    assert!(broker.revalidate_lease(&lease, ctx).is_ok());
    assert_eq!(
        broker.authorize(handle, ctx, Capability::NotesRead, &other).unwrap_err(),
        BrokerError::Denied
    );
    assert_eq!(
        broker.authorize(
            handle, HostContext::from_host_state(foreign, Some(active)),
            Capability::NotesRead, &allowed
        ).unwrap_err(),
        BrokerError::WrongVault
    );
    assert_eq!(
        broker.authorize(handle, ctx, Capability::NotesRead,
            &note(foreign, active, "Plan/day.md")).unwrap_err(),
        BrokerError::InvalidResource
    );
    assert_eq!(
        broker.revalidate_lease(&lease, HostContext::from_host_state(vault, Some(another))).unwrap_err(),
        BrokerError::StaleLease
    );
}

#[test]
fn selected_note_scopes_do_not_cover_unselected_search_hits() {
    let (broker, handle, vault, active) = fixture();
    let other = NoteId::new_for_host();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let selected = GrantScope::selected_notes([active]).unwrap();
    assert_eq!(GrantScope::selected_notes([]).unwrap_err(), BrokerError::InvalidScope);
    broker.grant_host_approved(handle, Capability::NotesSearch, selected).unwrap();
    // Search is checked per returned hit. A search adapter must filter all
    // snippets/paths/tags before returning a hit to a guest.
    assert!(broker.authorize(handle, ctx, Capability::NotesSearch,
        &note(vault, active, "Private/selected.md")).is_ok());
    assert_eq!(broker.authorize(handle, ctx, Capability::NotesSearch,
        &note(vault, other, "Private/unselected.md")).unwrap_err(), BrokerError::Denied);
    assert_eq!(broker.authorize(handle, ctx, Capability::NotesRead,
        &note(vault, active, "Private/selected.md")).unwrap_err(), BrokerError::Denied);
}

#[test]
fn segment_aware_folder_and_whole_vault_grants_are_separate() {
    let (broker, handle, vault, active) = fixture();
    let ctx = HostContext::from_host_state(vault, Some(active));
    broker.grant_host_approved(handle, Capability::NotesRead, GrantScope::folder("Work/Plans").unwrap()).unwrap();
    for path in ["Work/Plans", "Work/Plans/a.md", "Work/Plans/Nested/n.md"] {
        assert!(broker.authorize(handle, ctx, Capability::NotesRead,
            &note(vault, NoteId::new_for_host(), path)).is_ok(), "{path}");
    }
    for path in ["Work/Planning/a.md", "Work/PlansX.md", "Work/private.md", "work/Plans/a.md"] {
        assert_eq!(broker.authorize(handle, ctx, Capability::NotesRead,
            &note(vault, NoteId::new_for_host(), path)).unwrap_err(), BrokerError::Denied, "{path}");
    }
    // "Read all" is never implied by a folder grant.
    broker.grant_host_approved(handle, Capability::NotesPatch, GrantScope::WholeVault).unwrap();
    assert!(broker.authorize(handle, ctx, Capability::NotesPatch,
        &note(vault, NoteId::new_for_host(), "Work/private.md")).is_ok());
    assert_eq!(broker.authorize(handle, ctx, Capability::NotesSearch,
        &note(vault, NoteId::new_for_host(), "Work/private.md")).unwrap_err(), BrokerError::Denied);
}

#[test]
fn wrong_scope_classes_are_rejected_and_no_capability_is_ambient() {
    let (broker, handle, vault, active) = fixture();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let note = note(vault, active, "Home/n.md");
    for bad in [
        (Capability::NotesRead, GrantScope::Action),
        (Capability::UiCommand, GrantScope::WholeVault),
        (Capability::NetworkFetch, GrantScope::CurrentNote),
        (Capability::NotesRead, GrantScope::network_host("api.example.org").unwrap()),
    ] {
        assert_eq!(broker.grant_host_approved(handle, bad.0, bad.1).unwrap_err(),
            BrokerError::InvalidScope);
    }
    for action in [Capability::UiPanel, Capability::ClipboardRead, Capability::AiInvoke] {
        assert_eq!(broker.grant_host_approved(handle, action, GrantScope::Action).unwrap_err(),
            BrokerError::UndeclaredCapability);
    }
    assert_eq!(broker.authorize(handle, ctx, Capability::NotesCreate, &Resource::Ui).unwrap_err(),
        BrokerError::InvalidResource);
    assert_eq!(broker.authorize(handle, ctx, Capability::NotesRead, &note).unwrap_err(),
        BrokerError::Denied);
}

#[test]
fn network_host_must_be_both_declared_and_explicitly_granted() {
    let (broker, handle, vault, active) = fixture();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let target = Resource::Network(NetworkHost::parse_declared_host("api.example.org").unwrap());
    assert_eq!(broker.authorize(handle, ctx, Capability::NetworkFetch, &target).unwrap_err(), BrokerError::Denied);
    assert_eq!(broker.grant_host_approved(handle, Capability::NetworkFetch,
        GrantScope::network_host("outside.example.org").unwrap()).unwrap_err(),
        BrokerError::UndeclaredCapability);
    broker.grant_host_approved(handle, Capability::NetworkFetch,
        GrantScope::network_host("api.example.org").unwrap()).unwrap();
    assert!(broker.authorize(handle, ctx, Capability::NetworkFetch, &target).is_ok());
    let cdn = Resource::Network(NetworkHost::parse_declared_host("cdn.example.org").unwrap());
    assert_eq!(broker.authorize(handle, ctx, Capability::NetworkFetch, &cdn).unwrap_err(), BrokerError::Denied);
    for target in ["localhost", "*.example.org", "127.0.0.1", "api.example.org:443",
        "metadata.internal", "api.EXAMPLE.org"] {
        assert_eq!(NetworkHost::parse_declared_host(target).unwrap_err(),
            BrokerError::InvalidResource);
    }
    // A network permission alone cannot authorize note reads or raw sockets.
    assert_eq!(broker.authorize(handle, ctx, Capability::NetworkFetch,
        &Resource::HostAction).unwrap_err(), BrokerError::InvalidResource);
}

#[test]
fn revoked_permissions_and_removed_instances_invalidate_old_leases() {
    let (broker, handle, vault, active) = fixture();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let path = note(vault, active, "Work/a.md");
    broker.grant_host_approved(handle, Capability::NotesRead, GrantScope::CurrentNote).unwrap();
    let lease = broker.authorize(handle, ctx, Capability::NotesRead, &path).unwrap();
    broker.revoke_all_for_host(handle).unwrap();
    assert_eq!(broker.revalidate_lease(&lease, ctx).unwrap_err(), BrokerError::StaleLease);
    assert_eq!(broker.authorize(handle, ctx, Capability::NotesRead, &path).unwrap_err(), BrokerError::Denied);
    broker.grant_host_approved(handle, Capability::NotesRead, GrantScope::CurrentNote).unwrap();
    assert_eq!(broker.revalidate_lease(&lease, ctx).unwrap_err(), BrokerError::StaleLease);
    let new_lease = broker.authorize(handle, ctx, Capability::NotesRead, &path).unwrap();
    broker.unregister_host_instance(handle).unwrap();
    assert_eq!(broker.revalidate_lease(&new_lease, ctx).unwrap_err(), BrokerError::UnknownInstance);
}

#[test]
fn duplicate_grant_does_not_invalidate_live_lease_but_new_rights_do() {
    let (broker, handle, vault, active) = fixture();
    let ctx = HostContext::from_host_state(vault, Some(active));
    let path = note(vault, active, "Work/a.md");
    broker.grant_host_approved(handle, Capability::NotesRead, GrantScope::CurrentNote).unwrap();
    let lease = broker.authorize(handle, ctx, Capability::NotesRead, &path).unwrap();
    broker.grant_host_approved(handle, Capability::NotesRead, GrantScope::CurrentNote).unwrap();
    assert!(broker.revalidate_lease(&lease, ctx).is_ok());
    broker.grant_host_approved(handle, Capability::NotesSearch, GrantScope::WholeVault).unwrap();
    assert_eq!(broker.revalidate_lease(&lease, ctx).unwrap_err(), BrokerError::StaleLease);
}

#[test]
fn logical_path_validation_rejects_traversal_without_claiming_os_confinement() {
    for bad in ["/etc/passwd", "../file.md", "a/../file.md", "a//b.md",
        "a/./b.md", "a\\b.md", "C:/Windows/foo.md", "a/\0b.md", ""] {
        assert_eq!(RelativeVaultPath::parse_host_path(bad).unwrap_err(),
            BrokerError::InvalidResource, "{bad:?}");
    }
    assert!(RelativeVaultPath::parse_host_path("Исследования/книга.md").is_ok());
    assert!(RelativeVaultPath::parse_host_path("Work/Notes/My Task.md").is_ok());
}

#[test]
fn exhaustive_small_grant_matrix_never_leaks_cross_vault_or_unselected_notes() {
    let (broker, handle, vault, active) = fixture();
    let foreign = VaultId::new_for_host();
    let ids = [active, NoteId::new_for_host(), NoteId::new_for_host()];
    let paths = ["A/one.md", "A/two.md", "AB/three.md"];
    let ctx = HostContext::from_host_state(vault, Some(active));
    for scope in [
        GrantScope::CurrentNote,
        GrantScope::selected_notes([active]).unwrap(),
        GrantScope::folder("A").unwrap(),
        GrantScope::WholeVault,
    ] {
        broker.revoke_all_for_host(handle).unwrap();
        broker.grant_host_approved(handle, Capability::NotesRead, scope.clone()).unwrap();
        for i in 0..3 {
            for candidate_vault in [vault, foreign] {
                let resource = note(candidate_vault, ids[i], paths[i]);
                let granted = broker.authorize(handle, ctx, Capability::NotesRead, &resource).is_ok();
                let expected = candidate_vault == vault && match &scope {
                    GrantScope::CurrentNote => i == 0,
                    GrantScope::SelectedNotes(_) => i == 0,
                    GrantScope::Folder(_) => i < 2,
                    GrantScope::WholeVault => true,
                    _ => false,
                };
                assert_eq!(granted, expected, "i={i} scope={scope:?}");
            }
        }
    }
}

#[test]
fn every_unrequested_capability_cannot_be_authorized_by_any_grant() {
    let broker = PermissionBroker::new();
    let vault = VaultId::new_for_host();
    let m = manifest(&["ui.command"], &[]);
    let handle = broker.register_host_package(&m, [0xBE; 32], vault).unwrap();
    let ctx = HostContext::from_host_state(vault, None);
    let id = NoteId::new_for_host();
    let resource = note(vault, id, "Work/file.md");
    for action in [
        Capability::NotesRead, Capability::NotesSearch, Capability::NotesPatch,
        Capability::NotesCreate, Capability::NotesTrash, Capability::AttachmentsRead,
        Capability::AttachmentsWrite, Capability::EventsSubscribe,
    ] {
        assert_eq!(broker.grant_host_approved(handle, action, GrantScope::WholeVault).unwrap_err(),
            BrokerError::UndeclaredCapability);
        assert_eq!(broker.authorize(handle, ctx, action, &resource).unwrap_err(),
            BrokerError::UndeclaredCapability);
    }
}

#[test]
fn no_zero_digest_and_independent_instance_grants_even_for_same_manifest() {
    let broker = PermissionBroker::new();
    let vault = VaultId::new_for_host();
    let m = manifest(&["notes.read"], &[]);
    assert_eq!(broker.register_host_package(&m, [0; 32], vault).unwrap_err(),
        BrokerError::InvalidArtifact);
    let first = broker.register_host_package(&m, [1; 32], vault).unwrap();
    let second = broker.register_host_package(&m, [2; 32], vault).unwrap();
    let ctx = HostContext::from_host_state(vault, None);
    let res = note(vault, NoteId::new_for_host(), "A/x.md");
    broker.grant_host_approved(first, Capability::NotesRead, GrantScope::WholeVault).unwrap();
    assert!(broker.authorize(first, ctx, Capability::NotesRead, &res).is_ok());
    assert_eq!(broker.authorize(second, ctx, Capability::NotesRead, &res).unwrap_err(),
        BrokerError::Denied);
}
