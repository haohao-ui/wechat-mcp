use super::schema::{ExecutionMode, RootV1, SAFE_INT, canonical, decode_v1_root, require, sha256};
use super::store::{disk_error, stored_owner};
use super::*;
use std::path::Path;

#[derive(Clone, Copy)]
pub enum Installation {
    Fresh,
    LegacyUpgrade,
}

/// Only an explicit upgrade may seed these former defaults; never a runtime fallback.
pub fn legacy_seed(ctx: &AccountContext, service: bool, at: u64) -> Result<RoleRevision> {
    let unreadable = [
        (
            "unsupported_type",
            "这种格式暂时读不了（能读{supported_formats}），请对方转成这些格式再发",
        ),
        (
            "too_large",
            "文件超过{max_mb}MB上限，请对方压缩、拆分或截图关键部分再发",
        ),
        ("download_failed", "这次没能取到文件，请对方重发一次"),
        (
            "snapshot_changed",
            "取文件时聊天画面变了，没取到，请对方重发一次",
        ),
        (
            "extract_failed",
            "文件打不开或已加密，读不出内容，请对方另存为普通格式再发",
        ),
        (
            "empty",
            "文件里没有可读的文字（可能是扫描件或空文件），请对方发文字版或截图",
        ),
        (
            "transcribe_failed",
            "这条语音没能转成文字，请对方打字再说一次",
        ),
        ("unknown", "这次没能取到文件，请对方重发一次"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), Some(v.into())))
    .collect();
    RoleRevision {
        schema_version: 1,
        owner_account_id: ctx.account_id().into(),
        role_id: format!(
            "legacy_{}_{}",
            if service { "service" } else { "test" },
            &sha256(ctx.account_id().as_bytes())[..32]
        ),
        revision_id: "r1".into(),
        parent_revision_id: None,
        created_at: at,
        origin: "legacy_builtin_v1".into(),
        name: if service {
            "原内置客服"
        } else {
            "原内置测试"
        }
        .into(),
        persona: if service {
            "用户部署的个人微信AI客服助手"
        } else {
            "个人微信授权测试 Bot"
        }
        .into(),
        goals: if service {
            vec![
                "友好简洁地回应当前联系人，回复不超过100字".into(),
                "首次自我介绍时说明你是AI客服助手".into(),
                "信息不足时说明并询问".into(),
            ]
        } else {
            vec![
                "简短、自然的中文回复，不超过100字".into(),
                "说明这是自动回复测试".into(),
            ]
        },
        tone: if service {
            "友好简洁"
        } else {
            "简短自然"
        }
        .into(),
        business_rules: vec![],
        model_ref: "config.model_config".into(),
        knowledge_refs: vec![],
        templates: Templates {
            greeting: Some(
                if service {
                    "你好，我是AI客服助手。"
                } else {
                    "你好，这是自动回复测试。"
                }
                .into(),
            ),
            insufficient_information: Some("信息不足，请补充说明。".into()),
            unreadable,
            unreadable_missing_labels: Some("{labels}打不开，麻烦重发一下。".into()),
            unreadable_many_items: Some("有{count}个文件或图片打不开，麻烦重发一下。".into()),
        },
        context_budget: ContextBudget::default(),
        content_sha256: String::new(),
    }
    .seal()
}
impl Store {
    /// Caller must validate the legacy config with the existing strict reader first.
    /// Bytes are borrowed solely for provenance; they are never rewritten or logged.
    pub fn migrate(
        &self,
        ctx: &AccountContext,
        installation: Installation,
        config_bytes: &[u8],
        service: bool,
        at: u64,
    ) -> Result<u64> {
        let _lock = self.lock()?;
        let mut root = self.load()?;
        if matches!(installation, Installation::Fresh) {
            return Ok(root.generation);
        }
        let migration_id = format!("legacy-builtin-v1:{}", ctx.account_id());
        // A raw account ID may be 256 bytes; it remains metadata, never a path.
        require(migration_id.len() <= 274, "storage_capacity_exceeded")?;
        if root
            .migrations
            .iter()
            .any(|m| m.owner_account_id == ctx.account_id() && m.id == migration_id)
        {
            return Ok(root.generation);
        }
        require(
            at <= SAFE_INT && root.generation < SAFE_INT,
            "invalid_arguments",
        )?;
        require(
            root.migrations
                .iter()
                .filter(|m| m.owner_account_id == ctx.account_id())
                .count()
                < 4,
            "storage_capacity_exceeded",
        )?;
        // No accidental activation on a fresh/customized account whose old deployment
        // provenance has not been established by the upgrading caller.
        require(
            !root
                .accounts
                .iter()
                .any(|a| a.account_id == ctx.account_id()),
            "revision_conflict",
        )?;
        let config: serde_json::Value = serde_json::from_slice(config_bytes)?;
        let mode = config
            .pointer("/brain/mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("builtin");
        require(["builtin", "external"].contains(&mode), "invalid_arguments")?;
        let fallback = config
            .pointer("/brain/external/timeout_action")
            .and_then(serde_json::Value::as_str)
            == Some("builtin");
        let mut r = legacy_seed(ctx, if mode == "external" { true } else { service }, at)?;
        require(
            !root.roles.iter().any(|e| e.role_id == r.role_id),
            "revision_conflict",
        )?;
        require(
            root.roles
                .iter()
                .filter(|e| e.owner_account_id == ctx.account_id())
                .count()
                < 128,
            "storage_capacity_exceeded",
        )?;
        let generation = root.generation + 1;
        let a = Self::ensure_account(&mut root, ctx)?;
        if mode == "builtin" {
            a.active.default = Some(r.reference());
            a.draft = a.active.clone();
        } else if fallback {
            a.legacy_fallback = Some(r.reference());
        }
        let active = a.active.clone();
        root.roles.push(RoleEntry {
            owner_account_id: ctx.account_id().into(),
            role_id: r.role_id.clone(),
            draft_revision_id: r.revision_id.clone(),
            archived: false,
        });
        root.migrations.push(MigrationEntry {
            id: migration_id,
            owner_account_id: ctx.account_id().into(),
            source_config_sha256: sha256(config_bytes),
            seed_content_sha256: r.content_sha256.clone(),
            completed_generation: generation,
        });
        if mode == "builtin" {
            root.activation_history.push(ActivationEntry {
                account_id: ctx.account_id().into(),
                generation,
                active,
                at,
            });
        }
        root.generation = generation;
        root.validate()?;
        let relative = format!("brain-profiles/roles/{}/{}.json", r.role_id, r.revision_id);
        // After a pre-root failure, the exact same seed is a harmless existing orphan.
        // Preserve its immutable timestamp by reading it on retry in the same account.
        if self.exists(Path::new(&relative))? {
            let old: RoleRevision =
                super::schema::decode(&self.bytes(Path::new(&relative), 128 * 1024)?)?;
            old.validate()?;
            require(old.owner_account_id == ctx.account_id(), "not_found")?;
            r.created_at = old.created_at;
            r = r.seal()?;
            require(canonical(&r)? == canonical(&old)?, "integrity_mismatch")?;
            root.migrations
                .last_mut()
                .ok_or(Error("integrity_mismatch"))?
                .seed_content_sha256 = r.content_sha256.clone();
        }
        self.object(ctx, Path::new(&relative), &r)?;
        self.commit(&root)?;
        Ok(generation)
    }
}

impl Store {
    pub fn execution_mode(&self, ctx: &AccountContext) -> Result<ExecutionMode> {
        if !self.exists(Path::new("brain-bindings.json"))? {
            return Ok(ExecutionMode::Legacy);
        }
        let root = self.load()?;
        match root
            .accounts
            .iter()
            .find(|a| a.account_id == ctx.account_id())
        {
            Some(a) => Ok(a.execution_mode),
            None => Ok(ExecutionMode::Legacy),
        }
    }

    pub fn initialize(
        &self,
        ctx: &AccountContext,
        installation: Installation,
        validated_config_bytes: &[u8],
        service: bool,
        at: u64,
        upgrade_schema_from: Option<u32>,
    ) -> Result<u64> {
        if let Some(from_version) = upgrade_schema_from {
            require(from_version == 1, "schema_unsupported")?;
            return self.initialize_upgrade_v1(
                ctx,
                installation,
                validated_config_bytes,
                service,
                at,
            );
        }
        let _lock = self.lock()?;
        let mut root = self.load()?;
        if let Some(existing) = root
            .accounts
            .iter()
            .find(|a| a.account_id == ctx.account_id())
        {
            match existing.execution_mode {
                ExecutionMode::Legacy => {
                    return Err(Error("revision_conflict"));
                }
                ExecutionMode::Managed => {
                    for r_entry in root
                        .roles
                        .iter()
                        .filter(|r| r.owner_account_id == ctx.account_id())
                    {
                        let path = Self::object_path(
                            "roles",
                            &r_entry.role_id,
                            &r_entry.draft_revision_id,
                        )?;
                        let b = self.bytes(&path, 128 * 1024).map_err(|_| disk_error())?;
                        let r: RoleRevision =
                            super::schema::decode(&b).map_err(|_| disk_error())?;
                        stored_owner(
                            r.owner_account_id == r_entry.owner_account_id
                                && r.role_id == r_entry.role_id
                                && r.revision_id == r_entry.draft_revision_id,
                        )?;
                        r.validate().map_err(|_| disk_error())?;
                    }
                    match installation {
                        Installation::Fresh => {
                            let migration_id = format!("legacy-builtin-v1:{}", ctx.account_id());
                            if root.migrations.iter().any(|m| {
                                m.owner_account_id == ctx.account_id() && m.id == migration_id
                            }) {
                                return Err(Error("revision_conflict"));
                            }
                            return Ok(root.generation);
                        }
                        Installation::LegacyUpgrade => {
                            let migration_id = format!("legacy-builtin-v1:{}", ctx.account_id());
                            let m = root
                                .migrations
                                .iter()
                                .find(|m| {
                                    m.owner_account_id == ctx.account_id() && m.id == migration_id
                                })
                                .ok_or(Error("revision_conflict"))?;
                            require(
                                m.source_config_sha256 == sha256(validated_config_bytes),
                                "revision_conflict",
                            )?;
                            return Ok(root.generation);
                        }
                    }
                }
            }
        }

        require(
            at <= SAFE_INT && root.generation < SAFE_INT,
            "invalid_arguments",
        )?;
        require(root.accounts.len() < 16, "storage_capacity_exceeded")?;

        match installation {
            Installation::Fresh => {
                let generation = root.generation + 1;
                root.accounts.push(AccountEntry {
                    account_id: ctx.account_id().into(),
                    active: BindingSet::default(),
                    draft: BindingSet::default(),
                    legacy_fallback: None,
                    execution_mode: ExecutionMode::Managed,
                });
                root.generation = generation;
                root.validate()?;
                self.commit(&root)?;
                Ok(generation)
            }
            Installation::LegacyUpgrade => {
                let migration_id = format!("legacy-builtin-v1:{}", ctx.account_id());
                require(migration_id.len() <= 274, "storage_capacity_exceeded")?;
                require(
                    root.migrations
                        .iter()
                        .filter(|m| m.owner_account_id == ctx.account_id())
                        .count()
                        < 4,
                    "storage_capacity_exceeded",
                )?;
                let config: serde_json::Value = serde_json::from_slice(validated_config_bytes)?;
                let mode = config
                    .pointer("/brain/mode")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("builtin");
                require(["builtin", "external"].contains(&mode), "invalid_arguments")?;
                let fallback = config
                    .pointer("/brain/external/timeout_action")
                    .and_then(serde_json::Value::as_str)
                    == Some("builtin");
                let mut r = legacy_seed(ctx, if mode == "external" { true } else { service }, at)?;
                require(
                    !root.roles.iter().any(|e| e.role_id == r.role_id),
                    "revision_conflict",
                )?;
                require(
                    root.roles
                        .iter()
                        .filter(|e| e.owner_account_id == ctx.account_id())
                        .count()
                        < 128,
                    "storage_capacity_exceeded",
                )?;
                let generation = root.generation + 1;
                let mut a = AccountEntry {
                    account_id: ctx.account_id().into(),
                    active: BindingSet::default(),
                    draft: BindingSet::default(),
                    legacy_fallback: None,
                    execution_mode: ExecutionMode::Managed,
                };
                if mode == "builtin" {
                    a.active.default = Some(r.reference());
                    a.draft = a.active.clone();
                } else if fallback {
                    a.legacy_fallback = Some(r.reference());
                }
                let active = a.active.clone();
                root.accounts.push(a);
                root.roles.push(RoleEntry {
                    owner_account_id: ctx.account_id().into(),
                    role_id: r.role_id.clone(),
                    draft_revision_id: r.revision_id.clone(),
                    archived: false,
                });
                root.migrations.push(MigrationEntry {
                    id: migration_id,
                    owner_account_id: ctx.account_id().into(),
                    source_config_sha256: sha256(validated_config_bytes),
                    seed_content_sha256: r.content_sha256.clone(),
                    completed_generation: generation,
                });
                if mode == "builtin" {
                    root.activation_history.push(ActivationEntry {
                        account_id: ctx.account_id().into(),
                        generation,
                        active,
                        at,
                    });
                }
                root.generation = generation;
                root.validate()?;
                let relative = format!("brain-profiles/roles/{}/{}.json", r.role_id, r.revision_id);
                if self.exists(Path::new(&relative))? {
                    let old: RoleRevision =
                        super::schema::decode(&self.bytes(Path::new(&relative), 128 * 1024)?)?;
                    old.validate()?;
                    require(old.owner_account_id == ctx.account_id(), "not_found")?;
                    r.created_at = old.created_at;
                    r = r.seal()?;
                    require(canonical(&r)? == canonical(&old)?, "integrity_mismatch")?;
                    root.migrations
                        .last_mut()
                        .ok_or(Error("integrity_mismatch"))?
                        .seed_content_sha256 = r.content_sha256.clone();
                }
                self.object(ctx, Path::new(&relative), &r)?;
                self.commit(&root)?;
                Ok(generation)
            }
        }
    }

    fn initialize_upgrade_v1(
        &self,
        ctx: &AccountContext,
        installation: Installation,
        validated_config_bytes: &[u8],
        service: bool,
        at: u64,
    ) -> Result<u64> {
        let _lock = self.lock()?;
        require(self.exists(Path::new("brain-bindings.json"))?, "not_found")?;
        let raw = self.bytes(Path::new("brain-bindings.json"), 128 * 1024 * 1024)?;
        let root_v1: RootV1 = decode_v1_root(&raw)?;
        root_v1.validate()?;
        for role in &root_v1.roles {
            let path = Self::object_path("roles", &role.role_id, &role.draft_revision_id)?;
            let b = self.bytes(&path, 128 * 1024).map_err(|_| disk_error())?;
            let r: RoleRevision = super::schema::decode(&b).map_err(|_| disk_error())?;
            stored_owner(
                r.owner_account_id == role.owner_account_id
                    && r.role_id == role.role_id
                    && r.revision_id == role.draft_revision_id,
            )?;
            r.validate().map_err(|_| disk_error())?;
        }
        for k in &root_v1.knowledge {
            let path = Self::object_path("knowledge", &k.knowledge_id, &k.draft_revision_id)?;
            let b = self
                .bytes(&path, 2 * 1024 * 1024)
                .map_err(|_| disk_error())?;
            let r: KnowledgeRevision = super::schema::decode(&b).map_err(|_| disk_error())?;
            stored_owner(
                r.owner_account_id == k.owner_account_id
                    && r.knowledge_id == k.knowledge_id
                    && r.revision_id == k.draft_revision_id,
            )?;
            r.validate().map_err(|_| disk_error())?;
        }
        require(
            at <= SAFE_INT && root_v1.generation < SAFE_INT,
            "invalid_arguments",
        )?;

        let mut accounts: Vec<AccountEntry> = root_v1
            .accounts
            .into_iter()
            .map(|a| AccountEntry {
                account_id: a.account_id,
                active: a.active,
                draft: a.draft,
                legacy_fallback: a.legacy_fallback,
                execution_mode: ExecutionMode::Legacy,
            })
            .collect();

        let mut roles = root_v1.roles;
        let mut migrations = root_v1.migrations;
        let mut activation_history = root_v1.activation_history;
        let generation = root_v1.generation + 1;

        match installation {
            Installation::Fresh => {
                if accounts.iter().any(|a| a.account_id == ctx.account_id()) {
                    return Err(Error("revision_conflict"));
                }
                require(accounts.len() < 16, "storage_capacity_exceeded")?;
                accounts.push(AccountEntry {
                    account_id: ctx.account_id().into(),
                    active: BindingSet::default(),
                    draft: BindingSet::default(),
                    legacy_fallback: None,
                    execution_mode: ExecutionMode::Managed,
                });
            }
            Installation::LegacyUpgrade => {
                if let Some(a) = accounts
                    .iter_mut()
                    .find(|a| a.account_id == ctx.account_id())
                {
                    let migration_id = format!("legacy-builtin-v1:{}", ctx.account_id());
                    let m = migrations
                        .iter()
                        .find(|m| m.owner_account_id == ctx.account_id() && m.id == migration_id)
                        .ok_or(Error("revision_conflict"))?;
                    require(
                        m.source_config_sha256 == sha256(validated_config_bytes),
                        "revision_conflict",
                    )?;
                    a.execution_mode = ExecutionMode::Managed;
                } else {
                    let migration_id = format!("legacy-builtin-v1:{}", ctx.account_id());
                    require(migration_id.len() <= 274, "storage_capacity_exceeded")?;
                    require(accounts.len() < 16, "storage_capacity_exceeded")?;
                    let config: serde_json::Value = serde_json::from_slice(validated_config_bytes)?;
                    let mode = config
                        .pointer("/brain/mode")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("builtin");
                    require(["builtin", "external"].contains(&mode), "invalid_arguments")?;
                    let fallback = config
                        .pointer("/brain/external/timeout_action")
                        .and_then(serde_json::Value::as_str)
                        == Some("builtin");
                    let mut r =
                        legacy_seed(ctx, if mode == "external" { true } else { service }, at)?;
                    require(
                        !roles.iter().any(|e| e.role_id == r.role_id),
                        "revision_conflict",
                    )?;
                    let mut a = AccountEntry {
                        account_id: ctx.account_id().into(),
                        active: BindingSet::default(),
                        draft: BindingSet::default(),
                        legacy_fallback: None,
                        execution_mode: ExecutionMode::Managed,
                    };
                    if mode == "builtin" {
                        a.active.default = Some(r.reference());
                        a.draft = a.active.clone();
                    } else if fallback {
                        a.legacy_fallback = Some(r.reference());
                    }
                    let active = a.active.clone();
                    accounts.push(a);
                    roles.push(RoleEntry {
                        owner_account_id: ctx.account_id().into(),
                        role_id: r.role_id.clone(),
                        draft_revision_id: r.revision_id.clone(),
                        archived: false,
                    });
                    migrations.push(MigrationEntry {
                        id: migration_id,
                        owner_account_id: ctx.account_id().into(),
                        source_config_sha256: sha256(validated_config_bytes),
                        seed_content_sha256: r.content_sha256.clone(),
                        completed_generation: generation,
                    });
                    if mode == "builtin" {
                        activation_history.push(ActivationEntry {
                            account_id: ctx.account_id().into(),
                            generation,
                            active,
                            at,
                        });
                    }
                    let relative =
                        format!("brain-profiles/roles/{}/{}.json", r.role_id, r.revision_id);
                    if self.exists(Path::new(&relative))? {
                        let old: RoleRevision =
                            super::schema::decode(&self.bytes(Path::new(&relative), 128 * 1024)?)?;
                        old.validate()?;
                        require(old.owner_account_id == ctx.account_id(), "not_found")?;
                        r.created_at = old.created_at;
                        r = r.seal()?;
                        require(canonical(&r)? == canonical(&old)?, "integrity_mismatch")?;
                        migrations
                            .last_mut()
                            .ok_or(Error("integrity_mismatch"))?
                            .seed_content_sha256 = r.content_sha256.clone();
                    }
                    self.object(ctx, Path::new(&relative), &r)?;
                }
            }
        }

        let new_root = Root {
            schema_version: 2,
            generation,
            roles,
            knowledge: root_v1.knowledge,
            accounts,
            migrations,
            activation_history,
            request_results: root_v1.request_results,
        };
        new_root.validate()?;
        self.commit(&new_root)?;
        Ok(generation)
    }
}
