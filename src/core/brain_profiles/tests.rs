use super::*;
use std::{fs, path::PathBuf};
struct Fixture {
    path: PathBuf,
    store: Store,
    ctx: AccountContext,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("i1-profiles-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(path.starts_with(std::env::temp_dir()));
        let store = Store::open(&path).unwrap();
        Self {
            path,
            store,
            ctx: AccountContext::new("fixture_a".into()).unwrap(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
struct Access;
impl ContactAccess for Access {
    fn permits(&self, _: &str, c: &str, _: &ChatKind) -> bool {
        c != "denied"
    }
    fn family(&self, _: &str, c: &str) -> Vec<String> {
        vec![c.into()]
    }
}
fn req(g: u64, m: Mutation) -> WriteRequest {
    WriteRequest {
        expected_generation: g,
        request_id: uuid::Uuid::new_v4().to_string(),
        mutation: m,
    }
}
fn seed(f: &Fixture) -> RoleRevision {
    legacy_seed(&f.ctx, true, 0).unwrap()
}
#[test]
fn cas_draft_is_not_active_and_replay() {
    let f = Fixture::new();
    let r = seed(&f);
    let q = req(0, Mutation::SaveRole(r.clone()));
    let a = f.store.write(&f.ctx, &q, &Access, 0).unwrap();
    assert_eq!(a.generation, 1);
    assert_eq!(f.store.write(&f.ctx, &q, &Access, 1).unwrap(), a);
    assert_eq!(
        f.store
            .resolve(&f.ctx, "c", &ChatKind::Private, &Access)
            .unwrap_err()
            .0,
        "role_required"
    );
    assert_eq!(
        f.store.role(&f.ctx, &r.reference()).unwrap().content_sha256,
        r.content_sha256
    );
    let stale = req(0, Mutation::Activate);
    assert_eq!(
        f.store.write(&f.ctx, &stale, &Access, 0).unwrap_err().0,
        "revision_conflict"
    );
}
#[test]
fn migration_idempotent_and_config_untouched() {
    let f = Fixture::new();
    let config = br#"{ "brain":{"mode":"builtin"},"model_config":{"secret":"fixture"}}"#;
    let before = config.to_vec();
    assert_eq!(
        f.store
            .migrate(&f.ctx, Installation::LegacyUpgrade, config, true, 0)
            .unwrap(),
        1
    );
    let role = f
        .store
        .resolve(&f.ctx, "c", &ChatKind::Private, &Access)
        .unwrap();
    assert_eq!(role.templates.unreadable.len(), 8);
    assert!(
        role.templates
            .unreadable_missing_labels
            .unwrap()
            .contains("{labels}")
    );
    assert_eq!(
        f.store
            .migrate(&f.ctx, Installation::LegacyUpgrade, config, true, 1)
            .unwrap(),
        1
    );
    assert_eq!(config.as_slice(), before.as_slice());
}
#[test]
fn migration_fresh_has_no_role() {
    let f = Fixture::new();
    assert_eq!(
        f.store
            .migrate(&f.ctx, Installation::Fresh, b"{}", true, 0)
            .unwrap(),
        0
    );
    assert_eq!(f.store.generation().unwrap(), 0);
    assert!(!f.path.join("brain-bindings.json").exists());
}
#[test]
fn canonical_rfc8785_goldens() {
    let v = serde_json::json!({"\u{e000}":1,"\u{1f600}":2,"numbers":[333_333_333.333_333_29,1e30,4.50,2e-3,1e-27,-0.0]});
    assert_eq!(
        String::from_utf8(canonical(&v).unwrap()).unwrap(),
        "{\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27,0],\"😀\":2,\"\":1}"
    );
}
#[test]
fn cas_activation_rollback_and_dormant_permissions() {
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let b = BindingSet {
        default: Some(r.reference()),
        overrides: vec![],
    };
    f.store
        .write(&f.ctx, &req(1, Mutation::SaveBindings(b)), &Access, 0)
        .unwrap();
    f.store
        .write(&f.ctx, &req(2, Mutation::Activate), &Access, 0)
        .unwrap();
    assert_eq!(
        f.store
            .resolve(&f.ctx, "c", &ChatKind::Private, &Access)
            .unwrap()
            .revision_id,
        r.revision_id
    );
    assert_eq!(
        f.store
            .resolve(&f.ctx, "denied", &ChatKind::Private, &Access)
            .unwrap_err()
            .0,
        "not_found"
    );
    assert_eq!(
        f.store
            .write(
                &f.ctx,
                &req(3, Mutation::ArchiveRole { role_id: r.role_id }),
                &Access,
                0
            )
            .unwrap_err()
            .0,
        "role_in_use"
    );
}
#[test]
fn cas_unknown_schema_and_corruption_not_reset() {
    let f = Fixture::new();
    for bytes in [br#"{"schema_version":2}"#.as_slice(), b"{"] {
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(f.path.join("brain-bindings.json"))
                .unwrap();
            file.write_all(bytes).unwrap();
        }
        assert!(f.store.generation().is_err());
        assert!(
            f.store
                .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0)
                .is_err()
        );
        assert_eq!(fs::read(f.path.join("brain-bindings.json")).unwrap(), bytes);
    }
}
#[test]
fn cas_account_hiding_before_replay_or_cas() {
    let f = Fixture::new();
    let r = seed(&f);
    let q = req(0, Mutation::SaveRole(r.clone()));
    f.store.write(&f.ctx, &q, &Access, 0).unwrap();
    let other = AccountContext::new("fixture_b".into()).unwrap();
    assert_eq!(
        f.store.role(&other, &r.reference()).unwrap_err().0,
        "not_found"
    );
    assert_eq!(
        f.store.write(&other, &q, &Access, 0).unwrap_err().0,
        "not_found"
    );
    let copy = req(
        0,
        Mutation::CopyRole {
            source: r.reference(),
            new_role_id: "new".into(),
        },
    );
    assert_eq!(
        f.store.write(&other, &copy, &Access, 0).unwrap_err().0,
        "not_found"
    );
}
#[test]
fn cas_request_conflict_and_operation_scope() {
    let f = Fixture::new();
    let r = seed(&f);
    let q = req(0, Mutation::SaveRole(r.clone()));
    f.store.write(&f.ctx, &q, &Access, 0).unwrap();
    let mut changed = q.clone();
    if let Mutation::SaveRole(ref mut r) = changed.mutation {
        r.name = "other".into();
        *r = r.clone().seal().unwrap();
    }
    assert_eq!(
        f.store.write(&f.ctx, &changed, &Access, 0).unwrap_err().0,
        "request_conflict"
    );
    let mut other = q;
    other.mutation = Mutation::Activate;
    other.expected_generation = 1;
    f.store.write(&f.ctx, &other, &Access, 0).unwrap();
}
#[test]
fn cas_immutable_parent_and_hash() {
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let mut next = r.clone();
    next.revision_id = "r2".into();
    next.parent_revision_id = Some("wrong".into());
    next = next.seal().unwrap();
    assert!(
        f.store
            .write(
                &f.ctx,
                &req(1, Mutation::SaveRole(next.clone())),
                &Access,
                0
            )
            .is_err()
    );
    next.parent_revision_id = Some(r.revision_id.clone());
    next = next.seal().unwrap();
    f.store
        .write(&f.ctx, &req(1, Mutation::SaveRole(next)), &Access, 0)
        .unwrap();
    let mut collision = r.clone();
    collision.name = "collision".into();
    collision.parent_revision_id = Some("r2".into());
    collision = collision.seal().unwrap();
    assert!(
        f.store
            .write(&f.ctx, &req(2, Mutation::SaveRole(collision)), &Access, 0)
            .is_err()
    );
    assert_eq!(f.store.role(&f.ctx, &r.reference()).unwrap().name, r.name);
}
#[test]
fn migration_external_modes_unchanged() {
    for action in ["builtin", "skip", "handoff"] {
        let f = Fixture::new();
        let bytes=format!(r#"{{"brain":{{"mode":"external","external":{{"timeout_action":"{action}","timeout_seconds":60,"queue_minutes":15}}}},"model_config":{{"backend":"mock"}}}}"#).into_bytes();
        let config = f.path.join("config.json");
        fs::write(&config, &bytes).unwrap();
        f.store
            .migrate(&f.ctx, Installation::LegacyUpgrade, &bytes, true, 0)
            .unwrap();
        let root: serde_json::Value =
            serde_json::from_slice(&fs::read(f.path.join("brain-bindings.json")).unwrap()).unwrap();
        assert!(root["accounts"][0]["active"]["default"].is_null());
        assert_eq!(
            root["accounts"][0]["legacy_fallback"].is_object(),
            action == "builtin"
        );
        assert_eq!(fs::read(config).unwrap(), bytes);
    }
}
#[test]
fn schema_unknown_missing_nullable_and_limits() {
    let f = Fixture::new();
    let r = seed(&f);
    let mut v = serde_json::to_value(&r).unwrap();
    v["extra"] = true.into();
    assert!(serde_json::from_value::<RoleRevision>(v).is_err());
    let mut v = serde_json::to_value(&r).unwrap();
    v.as_object_mut().unwrap().remove("parent_revision_id");
    assert!(serde_json::from_value::<RoleRevision>(v).is_err());
    let mut r = r;
    r.persona = "长".repeat(8001);
    assert_eq!(r.seal().unwrap_err().0, "storage_capacity_exceeded");
}
#[test]
fn schema_template_unknown_variable_and_knowledge_limit() {
    let f = Fixture::new();
    let mut r = seed(&f);
    r.templates.greeting = Some("{secret}".into());
    assert_eq!(r.seal().unwrap_err().0, "template_invalid");
    let k = KnowledgeRevision {
        schema_version: 1,
        owner_account_id: f.ctx.account_id().into(),
        knowledge_id: "k".into(),
        revision_id: "r1".into(),
        parent_revision_id: None,
        created_at: 0,
        title: "知识".into(),
        text: "x".repeat(1_048_577),
        content_sha256: String::new(),
        source_label: "用户摘要".into(),
    };
    assert_eq!(k.seal().unwrap_err().0, "storage_capacity_exceeded");
}
#[test]
fn cas_paths_and_symlinks() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = Fixture::new();
    symlink(&outside.path, f.path.join("brain-profiles")).unwrap();
    assert!(
        f.store
            .write(&f.ctx, &req(0, Mutation::SaveRole(seed(&f))), &Access, 0)
            .is_err()
    );
    assert!(!outside.path.join("roles").exists());
    let f = Fixture::new();
    let mut r = seed(&f);
    r.role_id = "../escape".into();
    assert!(r.seal().is_err());
    symlink(&outside.path, f.path.join("brain-bindings.json")).unwrap();
    assert!(f.store.generation().is_err());
}
#[test]
fn cas_faults_preserve_root_and_retry() {
    for stage in [
        "object_create",
        "object_write",
        "object_sync",
        "object_rename",
        "object_dir_sync",
        "root_create",
        "root_write",
        "root_sync",
        "root_rename",
        "root_dir_sync",
    ] {
        let mut f = Fixture::new();
        let r = seed(&f);
        let q = req(0, Mutation::SaveRole(r));
        f.store.fail = Some(stage);
        assert!(f.store.write(&f.ctx, &q, &Access, 0).is_err(), "{stage}");
        f.store.fail = None;
        let expected = if stage == "root_dir_sync" { 1 } else { 0 };
        assert_eq!(f.store.generation().unwrap(), expected, "{stage}");
        assert_eq!(
            f.store.write(&f.ctx, &q, &Access, 1).unwrap().generation,
            1,
            "{stage}"
        );
        assert_eq!(f.store.generation().unwrap(), 1);
    }
}
#[test]
fn migration_faults_retry_with_later_clock() {
    for stage in [
        "object_create",
        "object_write",
        "object_sync",
        "object_rename",
        "object_dir_sync",
        "root_create",
        "root_write",
        "root_sync",
        "root_rename",
        "root_dir_sync",
    ] {
        let mut f = Fixture::new();
        f.store.fail = Some(stage);
        assert!(
            f.store
                .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0)
                .is_err()
        );
        f.store.fail = None;
        assert_eq!(
            f.store
                .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 99)
                .unwrap(),
            1,
            "{stage}"
        );
    }
}
#[test]
fn cas_two_concurrent_writers_one_commit() {
    let f = Fixture::new();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let r = seed(&f);
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = f.path.clone();
            let ctx = f.ctx.clone();
            let q = req(0, Mutation::SaveRole(r.clone()));
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                barrier.wait();
                store.write(&ctx, &q, &Access, 0)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .next()
            .unwrap()
            .0,
        "revision_conflict"
    );
    assert_eq!(f.store.generation().unwrap(), 1);
}
fn knowledge(ctx: &AccountContext) -> KnowledgeRevision {
    KnowledgeRevision {
        schema_version: 1,
        owner_account_id: ctx.account_id().into(),
        knowledge_id: "knowledge_a".into(),
        revision_id: "r1".into(),
        parent_revision_id: None,
        created_at: 0,
        title: "人工摘要".into(),
        text: "这是明确由用户提供的知识摘要".into(),
        content_sha256: String::new(),
        source_label: "用户摘要，非完整原文".into(),
    }
    .seal()
    .unwrap()
}
#[test]
fn cas_knowledge_ownership_digest_and_parent() {
    let f = Fixture::new();
    let k = knowledge(&f.ctx);
    let kr = KnowledgeRef {
        knowledge_id: k.knowledge_id.clone(),
        revision_id: k.revision_id.clone(),
        content_sha256: k.content_sha256.clone(),
    };
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveKnowledge(k)), &Access, 0)
        .unwrap();
    let mut r = seed(&f);
    r.knowledge_refs = vec![kr.clone()];
    r = r.seal().unwrap();
    f.store
        .write(&f.ctx, &req(1, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    assert_eq!(
        f.store.knowledge(&f.ctx, &kr).unwrap().text,
        "这是明确由用户提供的知识摘要"
    );
    let other = AccountContext::new("fixture_b".into()).unwrap();
    let mut stolen = legacy_seed(&other, true, 0).unwrap();
    stolen.knowledge_refs = vec![kr.clone()];
    stolen = stolen.seal().unwrap();
    assert_eq!(
        f.store
            .write(&other, &req(2, Mutation::SaveRole(stolen)), &Access, 0)
            .unwrap_err()
            .0,
        "not_found"
    );
    r.revision_id = "r2".into();
    r.parent_revision_id = Some("r1".into());
    r.knowledge_refs[0].content_sha256 = "0".repeat(64);
    r = r.seal().unwrap();
    assert_eq!(
        f.store
            .write(&f.ctx, &req(2, Mutation::SaveRole(r)), &Access, 0)
            .unwrap_err()
            .0,
        "not_found"
    );
}
struct Family;
impl ContactAccess for Family {
    fn permits(&self, _: &str, _: &str, _: &ChatKind) -> bool {
        true
    }
    fn family(&self, _: &str, _: &str) -> Vec<String> {
        vec!["stable_a".into(), "stable_b".into()]
    }
}
#[test]
fn cas_stable_overrides_merge_ambiguity_and_rollback() {
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let mut second = r.clone();
    second.role_id = "role_b".into();
    second = second.seal().unwrap();
    f.store
        .write(
            &f.ctx,
            &req(1, Mutation::SaveRole(second.clone())),
            &Access,
            0,
        )
        .unwrap();
    let binding = BindingSet {
        default: Some(r.reference()),
        overrides: vec![
            Override {
                contact_id: "stable_a".into(),
                chat_kind: ChatKind::Private,
                role_id: r.role_id.clone(),
                revision_id: r.revision_id.clone(),
            },
            Override {
                contact_id: "stable_b".into(),
                chat_kind: ChatKind::Private,
                role_id: second.role_id.clone(),
                revision_id: second.revision_id.clone(),
            },
        ],
    };
    f.store
        .write(&f.ctx, &req(2, Mutation::SaveBindings(binding)), &Access, 0)
        .unwrap();
    f.store
        .write(&f.ctx, &req(3, Mutation::Activate), &Access, 0)
        .unwrap();
    assert_eq!(
        f.store
            .resolve(&f.ctx, "stable_b", &ChatKind::Private, &Access)
            .unwrap()
            .role_id,
        second.role_id
    );
    assert_eq!(
        f.store
            .resolve(&f.ctx, "stable_a", &ChatKind::Private, &Family)
            .unwrap_err()
            .0,
        "binding_ambiguous"
    );
    assert_eq!(
        f.store
            .resolve(&f.ctx, "stable_b", &ChatKind::Group, &Access)
            .unwrap()
            .role_id,
        r.role_id
    );
    f.store
        .write(
            &f.ctx,
            &req(4, Mutation::SaveBindings(BindingSet::default())),
            &Access,
            0,
        )
        .unwrap();
    f.store
        .write(&f.ctx, &req(5, Mutation::Activate), &Access, 0)
        .unwrap();
    assert_eq!(
        f.store
            .resolve(&f.ctx, "stable_b", &ChatKind::Private, &Access)
            .unwrap_err()
            .0,
        "role_required"
    );
    f.store
        .write(
            &f.ctx,
            &req(6, Mutation::Rollback { generation: 4 }),
            &Access,
            0,
        )
        .unwrap();
    assert_eq!(
        f.store
            .resolve(&f.ctx, "stable_b", &ChatKind::Private, &Access)
            .unwrap()
            .role_id,
        second.role_id
    );
}
#[test]
fn migration_complete_content_matches_design_fixture() {
    let f = Fixture::new();
    let r = seed(&f);
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/design/production-brain-examples/migration.json"
    ))
    .unwrap();
    let mut expected = fixture["after"]["role"].clone();
    let mut actual = serde_json::to_value(r).unwrap();
    for k in [
        "owner_account_id",
        "role_id",
        "revision_id",
        "content_sha256",
        "name",
    ] {
        expected.as_object_mut().unwrap().remove(k);
        actual.as_object_mut().unwrap().remove(k);
    }
    assert_eq!(actual, expected);
}
#[test]
fn cas_failed_root_read_does_not_disclose_other_owner() {
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let path = f.path.join(format!(
        "brain-profiles/roles/{}/{}.json",
        r.role_id, r.revision_id
    ));
    let mut v = serde_json::to_value(&r).unwrap();
    v["owner_account_id"] = "foreign".into();
    fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        f.store.role(&f.ctx, &r.reference()).unwrap_err().0,
        "not_found"
    );
}
#[test]
fn cas_replay_checks_persisted_object_owner_first() {
    let f = Fixture::new();
    let r = seed(&f);
    let q = req(0, Mutation::SaveRole(r.clone()));
    f.store.write(&f.ctx, &q, &Access, 0).unwrap();
    let path = f.path.join(format!(
        "brain-profiles/roles/{}/{}.json",
        r.role_id, r.revision_id
    ));
    let mut forged = r;
    forged.owner_account_id = "fixture_foreign".into();
    forged = forged.seal().unwrap();
    fs::write(path, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert_eq!(
        f.store.write(&f.ctx, &q, &Access, 0).unwrap_err().0,
        "not_found"
    );
    assert!(f.store.list(&f.ctx).unwrap().1.is_empty());
}
#[test]
fn cas_metadata_boundaries_and_retention() {
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let path = f.path.join("brain-bindings.json");
    let mut root: Root = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    root.generation = 1000;
    root.request_results = (1..=1000)
        .map(|n| RequestEntry {
            account_id: f.ctx.account_id().into(),
            operation: "brain-activate".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            request_sha256: "0".repeat(64),
            result: WriteResult {
                generation: n,
                ..WriteResult::default()
            },
            at: 0,
        })
        .collect();
    root.activation_history = (901..=1000)
        .map(|n| ActivationEntry {
            account_id: f.ctx.account_id().into(),
            generation: n,
            active: BindingSet::default(),
            at: 0,
        })
        .collect();
    fs::write(&path, serde_json::to_vec(&root).unwrap()).unwrap();
    f.store
        .write(&f.ctx, &req(1000, Mutation::Activate), &Access, 0)
        .unwrap();
    let root: Root = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(root.request_results.len(), 1000);
    assert_eq!(root.request_results[0].result.generation, 2);
    assert_eq!(root.activation_history.len(), 100);
    assert_eq!(root.activation_history[0].generation, 902);
    let before = fs::read(&path).unwrap();
    let too_many = BindingSet {
        default: None,
        overrides: (0..201)
            .map(|n| Override {
                contact_id: format!("c{n}"),
                chat_kind: ChatKind::Private,
                role_id: r.role_id.clone(),
                revision_id: r.revision_id.clone(),
            })
            .collect(),
    };
    assert_eq!(
        f.store
            .write(
                &f.ctx,
                &req(1001, Mutation::SaveBindings(too_many)),
                &Access,
                0
            )
            .unwrap_err()
            .0,
        "storage_capacity_exceeded"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn cas_private_permissions_and_symlink_lock() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(seed(&f))), &Access, 0)
        .unwrap();
    assert_eq!(
        fs::metadata(f.path.join("brain-bindings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(f.path.join("brain-profiles/roles"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let f = Fixture::new();
    fs::create_dir(f.path.join("brain-profiles")).unwrap();
    fs::set_permissions(
        f.path.join("brain-profiles"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let outside = Fixture::new();
    let sentinel = outside.path.join("sentinel");
    fs::write(&sentinel, b"unchanged").unwrap();
    symlink(&sentinel, f.path.join("brain-profiles/.write.lock")).unwrap();
    assert!(
        f.store
            .write(&f.ctx, &req(0, Mutation::SaveRole(seed(&f))), &Access, 0)
            .is_err()
    );
    assert_eq!(fs::read(sentinel).unwrap(), b"unchanged");
}
#[test]
fn cas_read_checks_parent_ownership() {
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let mut next = r.clone();
    next.revision_id = "r2".into();
    next.parent_revision_id = Some(r.revision_id.clone());
    next = next.seal().unwrap();
    f.store
        .write(
            &f.ctx,
            &req(1, Mutation::SaveRole(next.clone())),
            &Access,
            0,
        )
        .unwrap();
    let mut parent = r;
    parent.owner_account_id = "foreign".into();
    parent = parent.seal().unwrap();
    let p = f.path.join(format!(
        "brain-profiles/roles/{}/{}.json",
        parent.role_id, parent.revision_id
    ));
    fs::write(p, serde_json::to_vec(&parent).unwrap()).unwrap();
    assert_eq!(
        f.store.role(&f.ctx, &next.reference()).unwrap_err().0,
        "not_found"
    );
}

#[test]
fn migration_utf8_account_byte_boundaries() {
    for account in ["a".repeat(256), "界".repeat(85) + "a"] {
        let f = Fixture::new();
        let ctx = AccountContext::new(account.clone()).unwrap();
        assert_eq!(account.len(), 256);
        assert_eq!(
            f.store
                .migrate(&ctx, Installation::LegacyUpgrade, b"{}", true, 0)
                .unwrap(),
            1
        );
        assert_eq!(
            f.store
                .migrate(&ctx, Installation::LegacyUpgrade, b"{}", true, 1)
                .unwrap(),
            1
        );
        let root: Root =
            serde_json::from_slice(&fs::read(f.path.join("brain-bindings.json")).unwrap()).unwrap();
        assert_eq!(root.migrations[0].id.len(), 274);
        assert_eq!(root.migrations[0].owner_account_id, account);
    }
    assert!(AccountContext::new("a".repeat(257)).is_err());
    assert!(AccountContext::new("界".repeat(85) + "ab").is_err());
}
#[test]
fn cas_directory_replacement_cannot_write_outside() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let mut f = Fixture::new();
    let outside = Fixture::new();
    let roles = f.path.join("brain-profiles/roles").join(seed(&f).role_id);
    let moved = f.path.join("moved-role");
    let external = outside.path.clone();
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = observed.clone();
    let once = std::sync::atomic::AtomicBool::new(false);
    f.store.hook = Some(Box::new(move |stage| {
        if stage == "object_create" && !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
            fs::rename(&roles, &moved).unwrap();
            symlink(&external, &roles).unwrap();
        }
        if stage == "object_write" {
            seen.store(
                fs::read_dir(&external).unwrap().count() > 0,
                std::sync::atomic::Ordering::SeqCst,
            );
        }
    }));
    let result = f
        .store
        .write(&f.ctx, &req(0, Mutation::SaveRole(seed(&f))), &Access, 0);
    assert!(result.is_err());
    assert!(
        !observed.load(std::sync::atomic::Ordering::SeqCst),
        "temporary file created outside root"
    );
    assert_eq!(fs::read_dir(&outside.path).unwrap().count(), 0);
    assert!(!f.path.join("brain-bindings.json").exists());
    assert_eq!(
        fs::metadata(&outside.path).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
#[test]
fn cas_state_root_replacement_is_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let other = Fixture::new();
    let moved = f.path.with_extension("moved");
    fs::rename(&f.path, &moved).unwrap();
    symlink(&other.path, &f.path).unwrap();
    let result = f
        .store
        .write(&f.ctx, &req(0, Mutation::SaveRole(seed(&f))), &Access, 0);
    assert!(result.is_err());
    assert_eq!(fs::read_dir(&other.path).unwrap().count(), 0);
    fs::remove_file(&f.path).unwrap();
    fs::rename(moved, &f.path).unwrap();
}

#[test]
fn cas_error_releases_locks_and_file_descriptors() {
    let mut f = Fixture::new();
    let descriptors = || fs::read_dir("/dev/fd").unwrap().count();
    let before = descriptors();
    let request = req(0, Mutation::SaveRole(seed(&f)));
    for _ in 0..30 {
        f.store.fail = Some("object_sync");
        assert!(f.store.write(&f.ctx, &request, &Access, 0).is_err());
    }
    assert!(
        descriptors() <= before + 1,
        "descriptor leak after injected errors"
    );
    f.store.fail = None;
    let reopened = Store::open(&f.path).unwrap();
    assert_eq!(
        reopened
            .write(&f.ctx, &request, &Access, 0)
            .unwrap()
            .generation,
        1
    );
}

#[test]
fn cas_schema_fixture_and_boundary_quotas() {
    let f = Fixture::new();
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/design/production-brain-examples/root-schema.json"
    ))
    .unwrap();
    let root = serde_json::to_value(Root::default()).unwrap();
    let mut actual: Vec<_> = root
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    actual.sort_unstable();
    let mut expected: Vec<_> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    expected.sort_unstable();
    assert_eq!(actual, expected);
    let mut root = Root::default();
    root.accounts = (0..16)
        .map(|n| AccountEntry {
            account_id: format!("account_{n}"),
            active: BindingSet::default(),
            draft: BindingSet::default(),
            legacy_fallback: None,
            execution_mode: ExecutionMode::Legacy,
        })
        .collect();
    f.store.commit(&root).unwrap();
    let before = fs::read(f.path.join("brain-bindings.json")).unwrap();
    assert_eq!(
        f.store
            .write(&f.ctx, &req(0, Mutation::SaveRole(seed(&f))), &Access, 0)
            .unwrap_err()
            .0,
        "storage_capacity_exceeded"
    );
    assert_eq!(
        fs::read(f.path.join("brain-bindings.json")).unwrap(),
        before
    );
    let f = Fixture::new();
    let r = seed(&f);
    f.store
        .write(&f.ctx, &req(0, Mutation::SaveRole(r.clone())), &Access, 0)
        .unwrap();
    let before = fs::read(f.path.join("brain-bindings.json")).unwrap();
    let mut role = r;
    role.knowledge_refs = (0..33)
        .map(|n| KnowledgeRef {
            knowledge_id: format!("k{n}"),
            revision_id: "r1".into(),
            content_sha256: "0".repeat(64),
        })
        .collect();
    assert_eq!(role.seal().unwrap_err().0, "storage_capacity_exceeded");
    let mut root: Root = serde_json::from_slice(&before).unwrap();
    root.generation = SAFE_INT;
    f.store.commit(&root).unwrap();
    assert_eq!(
        f.store
            .write(&f.ctx, &req(SAFE_INT, Mutation::Activate), &Access, 0)
            .unwrap_err()
            .0,
        "storage_capacity_exceeded"
    );
}

#[test]
fn migration_test_role_and_failure_preserves_existing_config() {
    let f = Fixture::new();
    let config =
        br#"{ "mode":"test","brain":{"mode":"builtin"},"model_config":{"backend":"codex-exec"}}"#;
    let original = f.path.join("config.json");
    fs::write(&original, config).unwrap();
    f.store
        .migrate(&f.ctx, Installation::LegacyUpgrade, config, false, 0)
        .unwrap();
    let role = f
        .store
        .resolve(&f.ctx, "c", &ChatKind::Private, &Access)
        .unwrap();
    assert!(role.goals.iter().any(|g| g.contains("自动回复测试")));
    assert_eq!(role.origin, "legacy_builtin_v1");
    assert_eq!(fs::read(original).unwrap(), config);
}

#[test]
fn cas_quota_counts_orphans_once_and_never_evicts() {
    let mut f = Fixture::new();
    let role = seed(&f);
    let bytes = serde_json::to_vec(&role).unwrap();
    f.store.quota = bytes.len() as u64;
    let request = req(0, Mutation::SaveRole(role.clone()));
    f.store.fail = Some("root_sync");
    assert!(f.store.write(&f.ctx, &request, &Access, 0).is_err());
    f.store.fail = None;
    assert_eq!(
        f.store
            .write(&f.ctx, &request, &Access, 0)
            .unwrap()
            .generation,
        1
    );
    let before = fs::read(f.path.join("brain-bindings.json")).unwrap();
    let mut second = role;
    second.role_id = "second".into();
    second = second.seal().unwrap();
    assert_eq!(
        f.store
            .write(&f.ctx, &req(1, Mutation::SaveRole(second)), &Access, 0)
            .unwrap_err()
            .0,
        "storage_capacity_exceeded"
    );
    assert_eq!(
        fs::read(f.path.join("brain-bindings.json")).unwrap(),
        before
    );
}

struct NoContacts;
impl ContactAccess for NoContacts {
    fn permits(&self, _: &str, _: &str, _: &ChatKind) -> bool {
        false
    }
    fn family(&self, _: &str, _: &str) -> Vec<String> {
        vec![]
    }
}

#[test]
fn cas_rollback_restores_draft_and_active_without_touching_other_account() {
    let f = Fixture::new();
    let role = seed(&f);
    let write = |mutation| {
        f.store
            .write(
                &f.ctx,
                &req(f.store.generation().unwrap(), mutation),
                &Access,
                0,
            )
            .unwrap()
    };
    write(Mutation::SaveRole(role.clone()));
    let target = BindingSet {
        default: Some(role.reference()),
        overrides: vec![Override {
            contact_id: "target_contact".into(),
            chat_kind: ChatKind::Private,
            role_id: role.role_id.clone(),
            revision_id: role.revision_id.clone(),
        }],
    };
    write(Mutation::SaveBindings(target.clone()));
    let target_generation = write(Mutation::Activate).generation;
    let other = AccountContext::new("fixture_other".into()).unwrap();
    f.store
        .migrate(&other, Installation::LegacyUpgrade, b"{}", true, 0)
        .unwrap();
    write(Mutation::SaveBindings(BindingSet {
        default: None,
        overrides: target.overrides.clone(),
    }));
    write(Mutation::Activate);
    write(Mutation::SaveBindings(BindingSet::default()));
    let path = f.path.join("brain-bindings.json");
    let before = fs::read(&path).unwrap();
    let request = req(
        f.store.generation().unwrap(),
        Mutation::Rollback {
            generation: target_generation,
        },
    );
    assert_eq!(
        f.store
            .write(&f.ctx, &request, &NoContacts, 0)
            .unwrap_err()
            .0,
        "not_found"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        f.store.write(&other, &request, &Access, 0).unwrap_err().0,
        "not_found"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut stale = request.clone();
    stale.expected_generation -= 1;
    assert_eq!(
        f.store.write(&f.ctx, &stale, &Access, 0).unwrap_err().0,
        "revision_conflict"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    let result = f.store.write(&f.ctx, &request, &Access, 0).unwrap();
    let committed = fs::read(&path).unwrap();
    let old: Root = serde_json::from_slice(&before).unwrap();
    let new: Root = serde_json::from_slice(&committed).unwrap();
    let account = new
        .accounts
        .iter()
        .find(|a| a.account_id == f.ctx.account_id())
        .unwrap();
    assert_eq!(account.active, target);
    assert_eq!(
        account.draft, target,
        "rollback must rebuild draft as well as active"
    );
    let other_state = |root: &Root| {
        serde_json::to_value((
            root.accounts
                .iter()
                .filter(|a| a.account_id == other.account_id())
                .collect::<Vec<_>>(),
            root.roles
                .iter()
                .filter(|a| a.owner_account_id == other.account_id())
                .collect::<Vec<_>>(),
            root.migrations
                .iter()
                .filter(|a| a.owner_account_id == other.account_id())
                .collect::<Vec<_>>(),
            root.activation_history
                .iter()
                .filter(|a| a.account_id == other.account_id())
                .collect::<Vec<_>>(),
            root.request_results
                .iter()
                .filter(|a| a.account_id == other.account_id())
                .collect::<Vec<_>>(),
        ))
        .unwrap()
    };
    assert_eq!(other_state(&old), other_state(&new));
    assert_eq!(result.generation, old.generation + 1);
    assert_eq!(f.store.write(&f.ctx, &request, &Access, 1).unwrap(), result);
    assert_eq!(
        fs::read(&path).unwrap(),
        committed,
        "replay must not rewrite either binding"
    );
    assert_eq!(
        f.store
            .write(&f.ctx, &request, &NoContacts, 1)
            .unwrap_err()
            .0,
        "not_found"
    );
    assert_eq!(fs::read(&path).unwrap(), committed);
    write(Mutation::Activate);
    let (_, _, current) = f.store.list(&f.ctx).unwrap();
    assert_eq!(
        current.unwrap().active,
        target,
        "activate after rollback must not resurrect the old draft"
    );
}

#[test]
fn cas_concurrent_rollbacks_restore_both_bindings_once() {
    let f = Fixture::new();
    let target_generation = f
        .store
        .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0)
        .unwrap();
    let (_, _, account) = f.store.list(&f.ctx).unwrap();
    let target = account.unwrap().active;
    f.store
        .write(
            &f.ctx,
            &req(
                target_generation,
                Mutation::SaveBindings(BindingSet::default()),
            ),
            &Access,
            0,
        )
        .unwrap();
    let expected = f.store.generation().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = f.path.clone();
            let ctx = f.ctx.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                let request = req(
                    expected,
                    Mutation::Rollback {
                        generation: target_generation,
                    },
                );
                barrier.wait();
                store.write(&ctx, &request, &Access, 0)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results.iter().find_map(|r| r.as_ref().err()).unwrap().0,
        "revision_conflict"
    );
    assert_eq!(f.store.generation().unwrap(), expected + 1);
    let (_, _, account) = f.store.list(&f.ctx).unwrap();
    let account = account.unwrap();
    assert_eq!(account.active, target);
    assert_eq!(account.draft, target);
}

fn frozen_v1(f: &Fixture) -> Vec<u8> {
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(f.path.join("brain-bindings.json")).unwrap()).unwrap();
    value["schema_version"] = 1.into();
    for account in value["accounts"].as_array_mut().unwrap() {
        account.as_object_mut().unwrap().remove("execution_mode");
    }
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(f.path.join("brain-bindings.json"), &bytes).unwrap();
    bytes
}

#[test]
fn execution_fresh_managed_without_seed_and_repeat() {
    let f = Fixture::new();
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Legacy
    );
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::Fresh, b"{}", true, 0, None)
            .unwrap(),
        1
    );
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Managed
    );
    assert_eq!(
        f.store
            .resolve(&f.ctx, "c", &ChatKind::Private, &Access)
            .unwrap_err()
            .0,
        "role_required"
    );
    let bytes = fs::read(f.path.join("brain-bindings.json")).unwrap();
    let root: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(root["schema_version"], 2);
    assert_eq!(root["accounts"][0]["execution_mode"], "managed");
    assert_eq!(root["roles"].as_array().unwrap().len(), 0);
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::Fresh, b"{}", true, 1, None)
            .unwrap(),
        1
    );
    assert_eq!(fs::read(f.path.join("brain-bindings.json")).unwrap(), bytes);
    assert_eq!(
        f.store
            .execution_mode(&AccountContext::new("absent".into()).unwrap())
            .unwrap(),
        ExecutionMode::Legacy
    );
}

#[test]
fn execution_drafts_stay_legacy_activate_and_rollback_manage() {
    let f = Fixture::new();
    let role = seed(&f);
    f.store
        .write(
            &f.ctx,
            &req(0, Mutation::SaveRole(role.clone())),
            &Access,
            0,
        )
        .unwrap();
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Legacy
    );
    f.store
        .write(
            &f.ctx,
            &req(1, Mutation::SaveKnowledge(knowledge(&f.ctx))),
            &Access,
            0,
        )
        .unwrap();
    f.store
        .write(
            &f.ctx,
            &req(
                2,
                Mutation::SaveBindings(BindingSet {
                    default: Some(role.reference()),
                    overrides: vec![],
                }),
            ),
            &Access,
            0,
        )
        .unwrap();
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Legacy
    );
    let before = fs::read(f.path.join("brain-bindings.json")).unwrap();
    for installation in [Installation::Fresh, Installation::LegacyUpgrade] {
        assert_eq!(
            f.store
                .initialize(&f.ctx, installation, b"{}", true, 0, None)
                .unwrap_err()
                .0,
            "revision_conflict"
        );
        assert_eq!(
            fs::read(f.path.join("brain-bindings.json")).unwrap(),
            before
        );
    }
    f.store
        .write(&f.ctx, &req(3, Mutation::Activate), &Access, 0)
        .unwrap();
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Managed
    );
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0, None)
            .unwrap_err()
            .0,
        "revision_conflict"
    );
    f.store
        .write(
            &f.ctx,
            &req(4, Mutation::Rollback { generation: 4 }),
            &Access,
            0,
        )
        .unwrap();
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Managed
    );
}

#[test]
fn execution_legacy_initialize_repeat_checks_source_and_reachable_objects() {
    let f = Fixture::new();
    let config=br#"{"brain":{"mode":"external","external":{"timeout_action":"builtin","timeout_seconds":60}},"model_config":{"backend":"mock"}}"#;
    f.store
        .initialize(&f.ctx, Installation::LegacyUpgrade, config, true, 0, None)
        .unwrap();
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Managed
    );
    let before = fs::read(f.path.join("brain-bindings.json")).unwrap();
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 1, None)
            .unwrap_err()
            .0,
        "revision_conflict"
    );
    assert_eq!(
        fs::read(f.path.join("brain-bindings.json")).unwrap(),
        before
    );
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, config, true, 1, None)
            .unwrap(),
        1
    );
    let role = legacy_seed(&f.ctx, true, 0).unwrap();
    let path = f.path.join(format!(
        "brain-profiles/roles/{}/{}.json",
        role.role_id, role.revision_id
    ));
    fs::write(path, b"{}").unwrap();
    assert!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, config, true, 1, None)
            .is_err()
    );
    assert!(
        f.store
            .initialize(&f.ctx, Installation::Fresh, config, true, 1, None)
            .is_err()
    );
    assert_eq!(
        fs::read(f.path.join("brain-bindings.json")).unwrap(),
        before
    );
}

#[test]
fn execution_v1_requires_explicit_upgrade_and_preserves_other_account() {
    let f = Fixture::new();
    f.store
        .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0)
        .unwrap();
    let original = legacy_seed(&f.ctx, true, 0).unwrap();
    let object = f.path.join(format!(
        "brain-profiles/roles/{}/{}.json",
        original.role_id, original.revision_id
    ));
    let object_before = fs::read(&object).unwrap();
    let before = frozen_v1(&f);
    assert_eq!(f.store.generation().unwrap_err().0, "schema_unsupported");
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap_err().0,
        "schema_unsupported"
    );
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 1, None)
            .unwrap_err()
            .0,
        "schema_unsupported"
    );
    assert_eq!(
        fs::read(f.path.join("brain-bindings.json")).unwrap(),
        before
    );
    let other = AccountContext::new("new_account".into()).unwrap();
    assert_eq!(
        f.store
            .initialize(&other, Installation::Fresh, b"{}", true, 1, Some(1))
            .unwrap(),
        2
    );
    assert_eq!(
        f.store.execution_mode(&other).unwrap(),
        ExecutionMode::Managed
    );
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Legacy
    );
    let mut old: serde_json::Value = serde_json::from_slice(&before).unwrap();
    let mut new: serde_json::Value =
        serde_json::from_slice(&fs::read(f.path.join("brain-bindings.json")).unwrap()).unwrap();
    new["accounts"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["account_id"] == f.ctx.account_id());
    new["accounts"][0]
        .as_object_mut()
        .unwrap()
        .remove("execution_mode");
    old.as_object_mut().unwrap().remove("schema_version");
    new.as_object_mut().unwrap().remove("schema_version");
    old.as_object_mut().unwrap().remove("generation");
    new.as_object_mut().unwrap().remove("generation");
    assert_eq!(old, new);
    assert_eq!(fs::read(object).unwrap(), object_before);
}

#[test]
fn execution_v1_current_migration_upgrade_no_reseed_or_draft_loss() {
    let f = Fixture::new();
    f.store
        .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0)
        .unwrap();
    f.store
        .write(
            &f.ctx,
            &req(1, Mutation::SaveBindings(BindingSet::default())),
            &Access,
            0,
        )
        .unwrap();
    let before = frozen_v1(&f);
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::Fresh, b"{}", true, 1, Some(1))
            .unwrap_err()
            .0,
        "revision_conflict"
    );
    assert_eq!(
        fs::read(f.path.join("brain-bindings.json")).unwrap(),
        before
    );
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 1, Some(1))
            .unwrap(),
        3
    );
    let (_, _, a) = f.store.list(&f.ctx).unwrap();
    let a = a.unwrap();
    assert!(a.active.default.is_some());
    assert_eq!(a.draft, BindingSet::default());
    assert_eq!(
        f.store.execution_mode(&f.ctx).unwrap(),
        ExecutionMode::Managed
    );
    let bytes = fs::read(f.path.join("brain-bindings.json")).unwrap();
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 2, None)
            .unwrap(),
        3
    );
    assert_eq!(fs::read(f.path.join("brain-bindings.json")).unwrap(), bytes);
}

#[test]
fn execution_v1_rejects_unknown_closed_fields_and_corrupt_objects() {
    for corruption in ["unknown_root", "unknown_account", "object"] {
        let f = Fixture::new();
        f.store
            .migrate(&f.ctx, Installation::LegacyUpgrade, b"{}", true, 0)
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&frozen_v1(&f)).unwrap();
        match corruption {
            "unknown_root" => v["extra"] = true.into(),
            "unknown_account" => v["accounts"][0]["execution_mode"] = "managed".into(),
            _ => {
                let r = legacy_seed(&f.ctx, true, 0).unwrap();
                fs::write(
                    f.path.join(format!(
                        "brain-profiles/roles/{}/{}.json",
                        r.role_id, r.revision_id
                    )),
                    b"{}",
                )
                .unwrap();
            }
        }
        let bytes = serde_json::to_vec(&v).unwrap();
        fs::write(f.path.join("brain-bindings.json"), &bytes).unwrap();
        let other = AccountContext::new("new_account".into()).unwrap();
        assert!(
            f.store
                .initialize(&other, Installation::Fresh, b"{}", true, 1, Some(1))
                .is_err(),
            "{corruption}"
        );
        assert_eq!(fs::read(f.path.join("brain-bindings.json")).unwrap(), bytes);
    }
}

#[test]
fn execution_initialization_root_faults_and_unknown_schema() {
    for stage in [
        "root_create",
        "root_write",
        "root_sync",
        "root_rename",
        "root_dir_sync",
    ] {
        let mut f = Fixture::new();
        f.store.fail = Some(stage);
        assert!(
            f.store
                .initialize(&f.ctx, Installation::Fresh, b"{}", true, 0, None)
                .is_err()
        );
        f.store.fail = None;
        let expected = if stage == "root_dir_sync" {
            ExecutionMode::Managed
        } else {
            ExecutionMode::Legacy
        };
        assert_eq!(f.store.execution_mode(&f.ctx).unwrap(), expected);
        assert_eq!(
            f.store
                .initialize(&f.ctx, Installation::Fresh, b"{}", true, 1, None)
                .unwrap(),
            1
        );
    }
    let f = Fixture::new();
    assert_eq!(
        f.store
            .initialize(&f.ctx, Installation::Fresh, b"{}", true, 0, Some(3))
            .unwrap_err()
            .0,
        "schema_unsupported"
    );
}
