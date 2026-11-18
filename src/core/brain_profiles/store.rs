use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone)]
pub struct AccountContext {
    account_id: String,
}
impl AccountContext {
    /// Construct only after resolving the authenticated effective account in the caller.
    /// Never deserialize or choose this context from a user's request body.
    /// # Errors
    /// Returns invalid_arguments for empty identifiers or identifiers over 256 UTF-8 bytes.
    pub fn new(account_id: String) -> Result<Self> {
        schema::require(schema::account(&account_id), "invalid_arguments")?;
        Ok(Self { account_id })
    }
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}
pub trait ContactAccess {
    fn permits(&self, account: &str, contact: &str, kind: &ChatKind) -> bool;
    fn family(&self, account: &str, contact: &str) -> Vec<String>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", content = "payload", deny_unknown_fields)]
pub enum Mutation {
    #[serde(rename = "brain-save-role")]
    SaveRole(RoleRevision),
    #[serde(rename = "brain-save-knowledge")]
    SaveKnowledge(KnowledgeRevision),
    #[serde(rename = "brain-copy-role")]
    CopyRole {
        source: RoleRef,
        new_role_id: String,
    },
    #[serde(rename = "brain-archive-role")]
    ArchiveRole { role_id: String },
    #[serde(rename = "brain-save-bindings")]
    SaveBindings(BindingSet),
    #[serde(rename = "brain-activate")]
    Activate,
    #[serde(rename = "brain-rollback")]
    Rollback { generation: u64 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteRequest {
    pub expected_generation: u64,
    pub request_id: String,
    pub mutation: Mutation,
}
use super::schema::{SAFE_INT, account, decode, decode_root, hex, id, require};
use fs2::FileExt;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
};

// StoreRoot must be a private directory supplied by the trusted caller. Nothing here
// discovers HOME, config paths, contacts, databases, network or GUI services.
pub struct Store {
    pub(crate) root: PathBuf,
    base: unix::Dir,
    #[cfg(test)]
    pub(crate) fail: Option<&'static str>,
    #[cfg(test)]
    pub(crate) quota: u64,
    #[cfg(test)]
    pub(crate) hook: Option<Box<dyn Fn(&str) + Send + Sync>>,
}
pub(crate) fn disk_error() -> Error {
    tracing::warn!(code = "integrity_mismatch", "brain profile object rejected");
    Error("not_found")
}
pub(crate) fn stored_owner(valid: bool) -> Result<()> {
    if valid { Ok(()) } else { Err(disk_error()) }
}
struct Temp<'a> {
    dir: &'a unix::Dir,
    name: String,
}
impl Drop for Temp<'_> {
    fn drop(&mut self) {
        let _ = self.dir.unlink(&self.name);
    }
}
impl Mutation {
    fn name(&self) -> &'static str {
        match self {
            Self::SaveRole(_) => "brain-save-role",
            Self::SaveKnowledge(_) => "brain-save-knowledge",
            Self::CopyRole { .. } => "brain-copy-role",
            Self::ArchiveRole { .. } => "brain-archive-role",
            Self::SaveBindings(_) => "brain-save-bindings",
            Self::Activate => "brain-activate",
            Self::Rollback { .. } => "brain-rollback",
        }
    }
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        require(fs::symlink_metadata(root)?.is_dir(), "store_unavailable")?;
        let root = fs::canonicalize(root)?;
        let base = unix::Dir(
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(&root)?,
        );
        require(
            base.0.metadata()?.permissions().mode() & 0o077 == 0,
            "store_unavailable",
        )?;
        let store = Self {
            root,
            base,
            #[cfg(test)]
            fail: None,
            #[cfg(test)]
            quota: 1024 * 1024 * 1024,
            #[cfg(test)]
            hook: None,
        };
        store.root_identity()?;
        Ok(store)
    }
    fn root_identity(&self) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        let current = fs::symlink_metadata(&self.root)?;
        let pinned = self.base.0.metadata()?;
        require(
            current.is_dir()
                && current.dev() == pinned.dev()
                && current.ino() == pinned.ino()
                && current.permissions().mode() & 0o077 == 0,
            "store_unavailable",
        )
    }
    fn fault(&self, _stage: &str) -> Result<()> {
        #[cfg(test)]
        if let Some(h) = &self.hook {
            h(_stage);
        }
        #[cfg(test)]
        if self.fail == Some(_stage) {
            return Err(Error("store_unavailable"));
        }
        Ok(())
    }
    fn directory(&self, relative: &Path, create: bool) -> std::io::Result<unix::Dir> {
        self.root_identity().map_err(std::io::Error::other)?;
        let mut dir = self.base.duplicate()?;
        for c in relative.components() {
            let std::path::Component::Normal(c) = c else {
                return Err(std::io::Error::other("invalid path"));
            };
            dir = dir.child(
                c.to_str()
                    .ok_or_else(|| std::io::Error::other("invalid path"))?,
                create,
            )?;
        }
        Ok(dir)
    }
    fn parent(&self, relative: &Path, create: bool) -> std::io::Result<(unix::Dir, String)> {
        let parent = relative
            .parent()
            .ok_or_else(|| std::io::Error::other("invalid path"))?;
        let name = relative
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| std::io::Error::other("invalid path"))?;
        Ok((self.directory(parent, create)?, name.into()))
    }
    pub(crate) fn exists(&self, relative: &Path) -> Result<bool> {
        match self
            .parent(relative, false)
            .and_then(|(d, n)| d.open(&n, libc::O_RDONLY))
        {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    pub(crate) fn lock(&self) -> Result<File> {
        let dir = self.directory(Path::new("brain-profiles"), true)?;
        let f = dir.open(".write.lock", libc::O_CREAT | libc::O_RDWR)?;
        f.lock_exclusive()?;
        require(
            dir.same(&self.directory(Path::new("brain-profiles"), false)?)?,
            "store_unavailable",
        )?;
        Ok(f)
    }
    pub(crate) fn bytes(&self, relative: &Path, limit: u64) -> Result<Vec<u8>> {
        let (dir, n) = self.parent(relative, false)?;
        let f = dir.open(&n, libc::O_RDONLY)?;
        require(f.metadata()?.len() <= limit, "storage_capacity_exceeded")?;
        let mut b = vec![];
        f.take(limit + 1).read_to_end(&mut b)?;
        require(b.len() as u64 <= limit, "storage_capacity_exceeded")?;
        Ok(b)
    }
    pub(crate) fn load(&self) -> Result<Root> {
        if !self.exists(Path::new("brain-bindings.json"))? {
            return Ok(Root::default());
        }
        let root: Root =
            decode_root(&self.bytes(Path::new("brain-bindings.json"), 128 * 1024 * 1024)?)?;
        root.validate()?;
        Ok(root)
    }
    /// Account-scoped retained history. Read one root snapshot and validate every
    /// referenced object through the same ownership/integrity checks as `role`.
    pub(crate) fn history(
        &self,
        ctx: &AccountContext,
        role_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        use serde_json::json;
        use std::collections::BTreeMap;
        let root = self.load()?;
        if let Some(id_) = role_id {
            require(id(id_), "invalid_arguments")?;
            require(
                root.roles
                    .iter()
                    .any(|r| r.owner_account_id == ctx.account_id && r.role_id == id_),
                "not_found",
            )?;
        }
        let account = root
            .accounts
            .iter()
            .find(|a| a.account_id == ctx.account_id);
        let roles: Vec<_> = root
            .roles
            .iter()
            .filter(|r| {
                r.owner_account_id == ctx.account_id && role_id.is_none_or(|id_| id_ == r.role_id)
            })
            .collect();
        let generations: Vec<_> = root
            .activation_history
            .iter()
            .filter(|h| h.account_id == ctx.account_id)
            .collect();
        let references = |b: &BindingSet| -> Vec<RoleRef> {
            b.default
                .iter()
                .cloned()
                .chain(b.overrides.iter().map(|o| RoleRef {
                    role_id: o.role_id.clone(),
                    revision_id: o.revision_id.clone(),
                }))
                .collect()
        };
        let active = account.map(|a| references(&a.active)).unwrap_or_default();
        let mut pending: Vec<RoleRef> = roles
            .iter()
            .map(|r| RoleRef {
                role_id: r.role_id.clone(),
                revision_id: r.draft_revision_id.clone(),
            })
            .collect();
        pending.extend(active.clone());
        if let Some(a) = account {
            pending.extend(references(&a.draft));
            pending.extend(a.legacy_fallback.iter().cloned());
        }
        for h in &generations {
            pending.extend(references(&h.active));
        }
        // Committed drafts can become detached after rollback. Include retained
        // successful writes, but never enumerate uncommitted orphan files.
        for request in root
            .request_results
            .iter()
            .filter(|r| r.account_id == ctx.account_id)
        {
            if let (Some(role_id), Some(revision_id)) =
                (&request.result.role_id, &request.result.revision_id)
            {
                pending.push(RoleRef {
                    role_id: role_id.clone(),
                    revision_id: revision_id.clone(),
                });
            }
        }
        let mut versions = BTreeMap::new();
        while let Some(r) = pending.pop() {
            if role_id.is_some_and(|id_| id_ != r.role_id) {
                continue;
            }
            let key = (r.role_id.clone(), r.revision_id.clone());
            if versions.contains_key(&key) {
                continue;
            }
            let content = self.role_from(&root, ctx, &r)?;
            let knowledge: Vec<_> = content
                .knowledge_refs
                .iter()
                .map(|k| self.knowledge_from(&root, ctx, k))
                .collect::<Result<_>>()?;
            let is_draft = roles
                .iter()
                .any(|e| e.role_id == r.role_id && e.draft_revision_id == r.revision_id);
            versions.insert(key, json!({"role_id": r.role_id, "revision_id": r.revision_id, "created_at": content.created_at,
                "is_active": active.contains(&r), "is_draft": is_draft, "content": content, "knowledge": knowledge}));
            if let Some(parent) = content.parent_revision_id {
                pending.push(RoleRef {
                    role_id: content.role_id,
                    revision_id: parent,
                });
            }
        }
        // The store retains activation snapshots and a bounded write-result log,
        // not a full draft binding snapshot at every edit. Preserve that boundary.
        let mut generation_rows = BTreeMap::new();
        for request in root
            .request_results
            .iter()
            .filter(|r| r.account_id == ctx.account_id)
        {
            generation_rows.insert(request.result.generation, json!({
                "generation": request.result.generation, "at": request.at, "operation": request.operation,
                "role_id": request.result.role_id, "revision_id": request.result.revision_id,
                "is_activation": false, "active": null, "draft": null
            }));
        }
        for activation in &generations {
            let row = generation_rows
                .entry(activation.generation)
                .or_insert_with(|| {
                    json!({
                        "generation": activation.generation, "at": activation.at, "operation": null,
                        "role_id": null, "revision_id": null, "draft": null
                    })
                });
            row["is_activation"] = json!(true);
            row["active"] = json!(activation.active);
        }
        if let Some(row) = generation_rows.get_mut(&root.generation) {
            row["draft"] = json!(account.map(|a| &a.draft));
        }
        let generations: Vec<_> = generation_rows.into_values().collect();
        let mut versions: Vec<_> = versions.into_values().collect();
        versions.sort_by_key(|v| std::cmp::Reverse(v["created_at"].as_u64().unwrap_or(0)));
        Ok(
            json!({"account_id": ctx.account_id(), "generation": root.generation,
            "active": account.map(|a| &a.active), "draft": account.map(|a| &a.draft),
            "roles": roles, "generations": generations, "versions": versions,
            "history_scope": "retained_activations_committed_writes_and_reachable_revisions"}),
        )
    }

    pub fn generation(&self) -> Result<u64> {
        Ok(self.load()?.generation)
    }
    pub(crate) fn object_path(kind: &str, id_: &str, revision: &str) -> Result<PathBuf> {
        require(id(id_) && id(revision), "invalid_arguments")?;
        Ok(PathBuf::from(format!(
            "brain-profiles/{kind}/{id_}/{revision}.json"
        )))
    }
    pub(crate) fn role_from(
        &self,
        root: &Root,
        ctx: &AccountContext,
        role: &RoleRef,
    ) -> Result<RoleRevision> {
        require(
            id(&role.role_id) && id(&role.revision_id),
            "invalid_arguments",
        )?;
        require(
            root.roles
                .iter()
                .any(|e| e.owner_account_id == ctx.account_id && e.role_id == role.role_id),
            "not_found",
        )?;
        let b = self
            .bytes(
                &Self::object_path("roles", &role.role_id, &role.revision_id)?,
                128 * 1024,
            )
            .map_err(|_| disk_error())?;
        let r: RoleRevision = decode(&b).map_err(|_| disk_error())?;
        stored_owner(
            r.owner_account_id == ctx.account_id
                && r.role_id == role.role_id
                && r.revision_id == role.revision_id,
        )?;
        r.validate().map_err(|_| disk_error())?;
        if let Some(parent) = &r.parent_revision_id {
            require(parent != &r.revision_id, "not_found")?;
            let p: RoleRevision = decode(
                &self
                    .bytes(&Self::object_path("roles", &r.role_id, parent)?, 128 * 1024)
                    .map_err(|_| disk_error())?,
            )
            .map_err(|_| disk_error())?;
            stored_owner(
                p.owner_account_id == ctx.account_id
                    && p.role_id == r.role_id
                    && p.revision_id == *parent,
            )?;
            p.validate().map_err(|_| disk_error())?;
        }
        for k in &r.knowledge_refs {
            self.knowledge_from(root, ctx, k)?;
        }
        Ok(r)
    }
    fn knowledge_from(
        &self,
        root: &Root,
        ctx: &AccountContext,
        k: &KnowledgeRef,
    ) -> Result<KnowledgeRevision> {
        require(
            id(&k.knowledge_id) && id(&k.revision_id),
            "invalid_arguments",
        )?;
        require(
            root.knowledge
                .iter()
                .any(|e| e.owner_account_id == ctx.account_id && e.knowledge_id == k.knowledge_id),
            "not_found",
        )?;
        let b = self
            .bytes(
                &Self::object_path("knowledge", &k.knowledge_id, &k.revision_id)?,
                2 * 1024 * 1024,
            )
            .map_err(|_| disk_error())?;
        let r: KnowledgeRevision = decode(&b).map_err(|_| disk_error())?;
        stored_owner(
            r.owner_account_id == ctx.account_id
                && r.knowledge_id == k.knowledge_id
                && r.revision_id == k.revision_id
                && r.content_sha256 == k.content_sha256,
        )?;
        r.validate().map_err(|_| disk_error())?;
        if let Some(parent) = &r.parent_revision_id {
            require(parent != &r.revision_id, "not_found")?;
            let p: KnowledgeRevision = decode(
                &self
                    .bytes(
                        &Self::object_path("knowledge", &r.knowledge_id, parent)?,
                        2 * 1024 * 1024,
                    )
                    .map_err(|_| disk_error())?,
            )
            .map_err(|_| disk_error())?;
            stored_owner(
                p.owner_account_id == ctx.account_id
                    && p.knowledge_id == r.knowledge_id
                    && p.revision_id == *parent,
            )?;
            p.validate().map_err(|_| disk_error())?;
        }
        Ok(r)
    }
    pub fn role(&self, ctx: &AccountContext, role: &RoleRef) -> Result<RoleRevision> {
        let root = self.load()?;
        self.role_from(&root, ctx, role)
    }
    pub fn knowledge(&self, ctx: &AccountContext, k: &KnowledgeRef) -> Result<KnowledgeRevision> {
        self.knowledge_from(&self.load()?, ctx, k)
    }
    pub fn list(
        &self,
        ctx: &AccountContext,
    ) -> Result<(u64, Vec<RoleEntry>, Option<AccountEntry>)> {
        let root = self.load()?;
        let roles = root
            .roles
            .iter()
            .filter(|r| {
                r.owner_account_id == ctx.account_id
                    && self
                        .role_from(
                            &root,
                            ctx,
                            &RoleRef {
                                role_id: r.role_id.clone(),
                                revision_id: r.draft_revision_id.clone(),
                            },
                        )
                        .is_ok()
            })
            .cloned()
            .collect();
        Ok((
            root.generation,
            roles,
            root.accounts
                .into_iter()
                .find(|a| a.account_id == ctx.account_id),
        ))
    }
    fn bindings(
        &self,
        root: &Root,
        ctx: &AccountContext,
        b: &BindingSet,
        access: Option<&dyn ContactAccess>,
    ) -> Result<()> {
        b.validate()?;
        for r in b
            .default
            .iter()
            .cloned()
            .chain(b.overrides.iter().map(|o| RoleRef {
                role_id: o.role_id.clone(),
                revision_id: o.revision_id.clone(),
            }))
        {
            self.role_from(root, ctx, &r)?;
            require(
                root.roles
                    .iter()
                    .any(|e| e.role_id == r.role_id && !e.archived),
                "not_found",
            )?;
        }
        if let Some(access) = access {
            for o in &b.overrides {
                require(
                    access.permits(&ctx.account_id, &o.contact_id, &o.chat_kind),
                    "not_found",
                )?;
            }
        }
        Ok(())
    }
    pub fn resolve(
        &self,
        ctx: &AccountContext,
        contact: &str,
        kind: &ChatKind,
        access: &dyn ContactAccess,
    ) -> Result<RoleRevision> {
        require(id(contact), "invalid_arguments")?;
        require(access.permits(&ctx.account_id, contact, kind), "not_found")?;
        let root = self.load()?;
        let a = root
            .accounts
            .iter()
            .find(|a| a.account_id == ctx.account_id)
            .ok_or(Error("role_required"))?;
        let family = access.family(&ctx.account_id, contact);
        let mut found = vec![];
        for o in &a.active.overrides {
            if o.chat_kind == *kind
                && (o.contact_id == contact || family.contains(&o.contact_id))
                && access.permits(&ctx.account_id, &o.contact_id, kind)
            {
                let r = RoleRef {
                    role_id: o.role_id.clone(),
                    revision_id: o.revision_id.clone(),
                };
                if !found.contains(&r) {
                    found.push(r);
                }
            }
        }
        require(found.len() <= 1, "binding_ambiguous")?;
        let r = found
            .first()
            .or(a.active.default.as_ref())
            .ok_or(Error("role_required"))?;
        self.role_from(&root, ctx, r)
    }
    fn authorize(
        &self,
        root: &Root,
        ctx: &AccountContext,
        m: &Mutation,
        access: &dyn ContactAccess,
    ) -> Result<()> {
        match m {
            Mutation::SaveRole(r) => {
                require(r.owner_account_id == ctx.account_id, "not_found")?;
                if let Some(e) = root.roles.iter().find(|e| e.role_id == r.role_id) {
                    require(e.owner_account_id == ctx.account_id, "not_found")?;
                    self.role_from(
                        root,
                        ctx,
                        &RoleRef {
                            role_id: e.role_id.clone(),
                            revision_id: e.draft_revision_id.clone(),
                        },
                    )?;
                    if self.exists(&Self::object_path("roles", &r.role_id, &r.revision_id)?)? {
                        self.role_from(root, ctx, &r.reference())?;
                    }
                }
                if let Some(p) = &r.parent_revision_id {
                    self.role_from(
                        root,
                        ctx,
                        &RoleRef {
                            role_id: r.role_id.clone(),
                            revision_id: p.clone(),
                        },
                    )?;
                }
                for k in &r.knowledge_refs {
                    self.knowledge_from(root, ctx, k)?;
                }
            }
            Mutation::SaveKnowledge(k) => {
                require(k.owner_account_id == ctx.account_id, "not_found")?;
                if let Some(e) = root
                    .knowledge
                    .iter()
                    .find(|e| e.knowledge_id == k.knowledge_id)
                {
                    require(e.owner_account_id == ctx.account_id, "not_found")?;
                    for revision in [&e.draft_revision_id, &k.revision_id] {
                        let path = Self::object_path("knowledge", &k.knowledge_id, revision)?;
                        if self.exists(&path)? {
                            let stored: KnowledgeRevision = decode(
                                &self
                                    .bytes(&path, 2 * 1024 * 1024)
                                    .map_err(|_| disk_error())?,
                            )
                            .map_err(|_| disk_error())?;
                            self.knowledge_from(
                                root,
                                ctx,
                                &KnowledgeRef {
                                    knowledge_id: k.knowledge_id.clone(),
                                    revision_id: revision.clone(),
                                    content_sha256: stored.content_sha256,
                                },
                            )?;
                        }
                    }
                }
                if let Some(p) = &k.parent_revision_id {
                    let path = Self::object_path("knowledge", &k.knowledge_id, p)?;
                    require(
                        root.knowledge.iter().any(|e| {
                            e.knowledge_id == k.knowledge_id && e.owner_account_id == ctx.account_id
                        }),
                        "not_found",
                    )?;
                    let parent: KnowledgeRevision = decode(
                        &self
                            .bytes(&path, 2 * 1024 * 1024)
                            .map_err(|_| disk_error())?,
                    )
                    .map_err(|_| disk_error())?;
                    require(
                        parent.owner_account_id == ctx.account_id
                            && parent.knowledge_id == k.knowledge_id
                            && parent.revision_id == *p,
                        "not_found",
                    )?;
                    parent.validate().map_err(|_| disk_error())?;
                }
            }
            Mutation::CopyRole {
                source,
                new_role_id,
            } => {
                self.role_from(root, ctx, source)?;
                require(id(new_role_id), "invalid_arguments")?;
                if let Some(e) = root.roles.iter().find(|e| e.role_id == *new_role_id) {
                    require(e.owner_account_id == ctx.account_id, "not_found")?;
                }
            }
            Mutation::ArchiveRole { role_id } => {
                let e = root
                    .roles
                    .iter()
                    .find(|e| e.owner_account_id == ctx.account_id && e.role_id == *role_id)
                    .ok_or(Error("not_found"))?;
                self.role_from(
                    root,
                    ctx,
                    &RoleRef {
                        role_id: role_id.clone(),
                        revision_id: e.draft_revision_id.clone(),
                    },
                )?;
            }
            Mutation::SaveBindings(b) => self.bindings(root, ctx, b, Some(access))?,
            Mutation::Activate => {
                if let Some(a) = root
                    .accounts
                    .iter()
                    .find(|a| a.account_id == ctx.account_id)
                {
                    self.bindings(root, ctx, &a.draft, Some(access))?;
                }
            }
            Mutation::Rollback { generation } => {
                let h = root
                    .activation_history
                    .iter()
                    .find(|h| h.account_id == ctx.account_id && h.generation == *generation)
                    .ok_or(Error("not_found"))?;
                self.bindings(root, ctx, &h.active, Some(access))?;
            }
        }
        Ok(())
    }
    pub(crate) fn object<T: Serialize>(
        &self,
        ctx: &AccountContext,
        relative: &Path,
        object: &T,
    ) -> Result<()> {
        let b = serde_json::to_vec(object)?;
        let extra = if self.exists(relative)? {
            require(
                self.bytes(relative, 2 * 1024 * 1024)? == b,
                "integrity_mismatch",
            )?;
            0
        } else {
            b.len() as u64
        };
        self.capacity(ctx, extra)?;
        self.persist(relative, &b, false)
    }
    fn capacity(&self, ctx: &AccountContext, extra: u64) -> Result<()> {
        let mut size = extra;
        for kind in ["roles", "knowledge"] {
            let relative = PathBuf::from(format!("brain-profiles/{kind}"));
            let dir = match self.directory(&relative, false) {
                Ok(d) => d,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            for n in dir.names()? {
                require(id(&n), "store_unavailable")?;
                let objects = dir.child(&n, false)?;
                for object in objects.names()? {
                    if object.starts_with('.') && object.ends_with(".tmp") {
                        continue;
                    }
                    let revision = object
                        .strip_suffix(".json")
                        .ok_or(Error("store_unavailable"))?;
                    require(id(revision), "store_unavailable")?;
                    let f = objects.open(&object, libc::O_RDONLY)?;
                    let len = f.metadata()?.len();
                    require(len <= 2 * 1024 * 1024, "storage_capacity_exceeded")?;
                    let mut bytes = vec![];
                    f.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                    if value["owner_account_id"].as_str() == Some(&ctx.account_id) {
                        size = size
                            .checked_add(len)
                            .ok_or(Error("storage_capacity_exceeded"))?;
                    }
                }
            }
        }
        #[cfg(not(test))]
        let limit = 1024 * 1024 * 1024;
        #[cfg(test)]
        let limit = self.quota;
        require(size <= limit, "storage_capacity_exceeded")
    }
    fn persist(&self, relative: &Path, bytes: &[u8], replace: bool) -> Result<()> {
        let (dir, name) = self.parent(relative, true)?;
        if !replace && self.exists(relative)? {
            require(
                self.bytes(relative, 2 * 1024 * 1024)? == bytes,
                "integrity_mismatch",
            )?;
            return Ok(());
        }
        let prefix = if replace { "root" } else { "object" };
        let temp = Temp {
            dir: &dir,
            name: format!(".{}.tmp", uuid::Uuid::new_v4().simple()),
        };
        self.fault(&format!("{prefix}_create"))?;
        let mut file = dir.open(&temp.name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        self.fault(&format!("{prefix}_write"))?;
        file.write_all(bytes)?;
        self.fault(&format!("{prefix}_sync"))?;
        file.sync_all()?;
        self.fault(&format!("{prefix}_rename"))?;
        require(
            dir.same(
                &self.directory(relative.parent().ok_or(Error("invalid_arguments"))?, false)?,
            )?,
            "store_unavailable",
        )?;
        match dir.publish(&temp.name, &name, replace) {
            Ok(()) => {}
            Err(e) if !replace && e.kind() == std::io::ErrorKind::AlreadyExists => require(
                self.bytes(relative, 2 * 1024 * 1024)? == bytes,
                "integrity_mismatch",
            )?,
            Err(e) => return Err(e.into()),
        }
        self.fault(&format!("{prefix}_dir_sync"))?;
        dir.0.sync_all()?;
        Ok(())
    }
    pub(crate) fn commit(&self, root: &Root) -> Result<()> {
        root.validate()?;
        let b = serde_json::to_vec(root)?;
        require(b.len() <= 128 * 1024 * 1024, "storage_capacity_exceeded")?;
        self.persist(Path::new("brain-bindings.json"), &b, true)
    }
    pub(crate) fn ensure_account<'a>(
        root: &'a mut Root,
        ctx: &AccountContext,
    ) -> Result<&'a mut AccountEntry> {
        if !root.accounts.iter().any(|a| a.account_id == ctx.account_id) {
            require(root.accounts.len() < 16, "storage_capacity_exceeded")?;
            root.accounts.push(AccountEntry {
                account_id: ctx.account_id.clone(),
                active: BindingSet::default(),
                draft: BindingSet::default(),
                legacy_fallback: None,
                execution_mode: ExecutionMode::Legacy,
            });
        }
        root.accounts
            .iter_mut()
            .find(|a| a.account_id == ctx.account_id)
            .ok_or(Error("not_found"))
    }
    fn save_role(&self, root: &mut Root, ctx: &AccountContext, r: &RoleRevision) -> Result<()> {
        r.validate()?;
        if let Some(e) = root.roles.iter().find(|e| e.role_id == r.role_id) {
            require(
                !e.archived && r.parent_revision_id.as_ref() == Some(&e.draft_revision_id),
                "revision_conflict",
            )?;
        } else {
            require(r.parent_revision_id.is_none(), "not_found")?;
            require(
                root.roles
                    .iter()
                    .filter(|e| e.owner_account_id == ctx.account_id)
                    .count()
                    < 128,
                "storage_capacity_exceeded",
            )?;
        }
        let path = Self::object_path("roles", &r.role_id, &r.revision_id)?;
        if self.exists(&path)? {
            // Only a byte-identical orphan from a failed root commit may be reused.
            require(
                !root
                    .roles
                    .iter()
                    .any(|e| e.role_id == r.role_id && e.draft_revision_id == r.revision_id),
                "revision_conflict",
            )?;
        }
        self.object(ctx, &path, r)?;
        if let Some(e) = root.roles.iter_mut().find(|e| e.role_id == r.role_id) {
            e.draft_revision_id = r.revision_id.clone();
        } else {
            root.roles.push(RoleEntry {
                owner_account_id: ctx.account_id.clone(),
                role_id: r.role_id.clone(),
                draft_revision_id: r.revision_id.clone(),
                archived: false,
            });
        }
        Ok(())
    }
    pub fn write(
        &self,
        ctx: &AccountContext,
        request: &WriteRequest,
        access: &dyn ContactAccess,
        at: u64,
    ) -> Result<WriteResult> {
        let _lock = self.lock()?;
        let mut root = self.load()?;
        self.authorize(&root, ctx, &request.mutation, access)?;
        match &request.mutation {
            Mutation::SaveRole(r) => r.validate()?,
            Mutation::SaveKnowledge(k) => k.validate()?,
            _ => {}
        }
        require(
            request.expected_generation <= SAFE_INT
                && at <= SAFE_INT
                && uuid::Uuid::parse_str(&request.request_id)
                    .is_ok_and(|u| u.to_string() == request.request_id),
            "invalid_arguments",
        )?;
        let operation = request.mutation.name();
        let hash = sha256(&canonical(&(ctx.account_id(), operation, request))?);
        if let Some(old) = root.request_results.iter().find(|e| {
            e.account_id == ctx.account_id
                && e.operation == operation
                && e.request_id == request.request_id
        }) {
            require(old.request_sha256 == hash, "request_conflict")?;
            return Ok(old.result.clone());
        }
        require(
            root.generation == request.expected_generation,
            "revision_conflict",
        )?;
        require(root.generation < SAFE_INT, "storage_capacity_exceeded")?;
        let generation = root.generation + 1;
        let mut result = WriteResult {
            generation,
            ..WriteResult::default()
        };
        Self::ensure_account(&mut root, ctx)?;
        match &request.mutation {
            Mutation::SaveRole(r) => {
                self.save_role(&mut root, ctx, r)?;
                result.role_id = Some(r.role_id.clone());
                result.revision_id = Some(r.revision_id.clone());
                result.content_sha256 = Some(r.content_sha256.clone());
            }
            Mutation::SaveKnowledge(k) => {
                k.validate()?;
                if let Some(e) = root
                    .knowledge
                    .iter()
                    .find(|e| e.knowledge_id == k.knowledge_id)
                {
                    require(
                        k.parent_revision_id.as_ref() == Some(&e.draft_revision_id),
                        "revision_conflict",
                    )?;
                } else {
                    require(k.parent_revision_id.is_none(), "not_found")?;
                    require(
                        root.knowledge
                            .iter()
                            .filter(|e| e.owner_account_id == ctx.account_id)
                            .count()
                            < 128,
                        "storage_capacity_exceeded",
                    )?;
                }
                let path = Self::object_path("knowledge", &k.knowledge_id, &k.revision_id)?;
                if self.exists(&path)? {
                    require(
                        !root.knowledge.iter().any(|e| {
                            e.knowledge_id == k.knowledge_id && e.draft_revision_id == k.revision_id
                        }),
                        "revision_conflict",
                    )?;
                }
                self.object(ctx, &path, k)?;
                if let Some(e) = root
                    .knowledge
                    .iter_mut()
                    .find(|e| e.knowledge_id == k.knowledge_id)
                {
                    e.draft_revision_id = k.revision_id.clone();
                } else {
                    root.knowledge.push(KnowledgeEntry {
                        owner_account_id: ctx.account_id.clone(),
                        knowledge_id: k.knowledge_id.clone(),
                        draft_revision_id: k.revision_id.clone(),
                    });
                }
                result.knowledge_id = Some(k.knowledge_id.clone());
                result.revision_id = Some(k.revision_id.clone());
                result.content_sha256 = Some(k.content_sha256.clone());
            }
            Mutation::CopyRole {
                source,
                new_role_id,
            } => {
                require(
                    !root.roles.iter().any(|e| e.role_id == *new_role_id),
                    "revision_conflict",
                )?;
                let mut r = self.role_from(&root, ctx, source)?;
                r.role_id = new_role_id.clone();
                r.revision_id = uuid::Uuid::new_v4().simple().to_string();
                r.parent_revision_id = None;
                r.created_at = at;
                r.origin = "user".into();
                r = r.seal()?;
                for k in &r.knowledge_refs {
                    self.knowledge_from(&root, ctx, k)?;
                }
                self.save_role(&mut root, ctx, &r)?;
                result.role_id = Some(r.role_id);
                result.revision_id = Some(r.revision_id);
                result.content_sha256 = Some(r.content_sha256);
            }
            Mutation::ArchiveRole { role_id } => {
                for a in &root.accounts {
                    for b in [&a.active, &a.draft] {
                        require(
                            !b.default.as_ref().is_some_and(|r| r.role_id == *role_id)
                                && !b.overrides.iter().any(|o| o.role_id == *role_id),
                            "role_in_use",
                        )?;
                    }
                    require(
                        !a.legacy_fallback
                            .as_ref()
                            .is_some_and(|r| r.role_id == *role_id),
                        "role_in_use",
                    )?;
                }
                let e = root
                    .roles
                    .iter_mut()
                    .find(|e| e.owner_account_id == ctx.account_id && e.role_id == *role_id)
                    .ok_or(Error("not_found"))?;
                e.archived = true;
                result.role_id = Some(role_id.clone());
                result.archived = Some(true);
            }
            Mutation::SaveBindings(b) => {
                Self::ensure_account(&mut root, ctx)?.draft = b.clone();
            }
            Mutation::Activate | Mutation::Rollback { .. } => {
                let b = if let Mutation::Rollback { generation } = request.mutation {
                    root.activation_history
                        .iter()
                        .find(|h| h.account_id == ctx.account_id && h.generation == generation)
                        .ok_or(Error("not_found"))?
                        .active
                        .clone()
                } else {
                    Self::ensure_account(&mut root, ctx)?.draft.clone()
                };
                let account = Self::ensure_account(&mut root, ctx)?;
                if matches!(&request.mutation, Mutation::Rollback { .. }) {
                    account.draft = b.clone();
                }
                account.active = b.clone();
                account.execution_mode = ExecutionMode::Managed;
                root.activation_history.push(ActivationEntry {
                    account_id: ctx.account_id.clone(),
                    generation,
                    active: b,
                    at,
                });
                retain_account_history(&mut root.activation_history, &ctx.account_id, 100, |e| {
                    &e.account_id
                });
            }
        }
        root.generation = generation;
        root.request_results.push(RequestEntry {
            account_id: ctx.account_id.clone(),
            operation: operation.into(),
            request_id: request.request_id.clone(),
            request_sha256: hash,
            result: result.clone(),
            at,
        });
        retain_account_history(&mut root.request_results, &ctx.account_id, 1000, |e| {
            &e.account_id
        });
        self.commit(&root)?;
        Ok(result)
    }
}

fn retain_account_history<T>(
    items: &mut Vec<T>,
    account: &str,
    limit: usize,
    key: impl Fn(&T) -> &str,
) {
    let mut remove = items
        .iter()
        .filter(|x| key(x) == account)
        .count()
        .saturating_sub(limit);
    items.retain(|x| {
        if key(x) == account && remove > 0 {
            remove -= 1;
            false
        } else {
            true
        }
    });
}
impl BindingSet {
    pub(crate) fn validate(&self) -> Result<()> {
        require(self.overrides.len() <= 200, "storage_capacity_exceeded")?;
        let mut seen = HashSet::new();
        if let Some(r) = &self.default {
            require(id(&r.role_id) && id(&r.revision_id), "invalid_arguments")?;
        }
        for o in &self.overrides {
            require(
                id(&o.contact_id)
                    && id(&o.role_id)
                    && id(&o.revision_id)
                    && seen.insert(&o.contact_id),
                "invalid_arguments",
            )?;
        }
        Ok(())
    }
}
impl Root {
    pub(crate) fn validate(&self) -> Result<()> {
        require(self.schema_version == 2, "schema_unsupported")?;
        require(self.generation <= SAFE_INT, "invalid_arguments")?;
        require(
            self.accounts.len() <= 16
                && self.roles.len() <= 2048
                && self.knowledge.len() <= 2048
                && self.migrations.len() <= 64
                && self.activation_history.len() <= 1600
                && self.request_results.len() <= 16000,
            "storage_capacity_exceeded",
        )?;
        let mut accounts = HashSet::new();
        for a in &self.accounts {
            require(
                account(&a.account_id) && accounts.insert(&a.account_id),
                "invalid_arguments",
            )?;
            a.active.validate()?;
            a.draft.validate()?;
            if let Some(r) = &a.legacy_fallback {
                require(id(&r.role_id) && id(&r.revision_id), "invalid_arguments")?;
            }
        }
        let mut ids = HashSet::new();
        for e in &self.roles {
            require(
                accounts.contains(&e.owner_account_id)
                    && id(&e.role_id)
                    && id(&e.draft_revision_id)
                    && ids.insert(&e.role_id),
                "integrity_mismatch",
            )?;
        }
        ids.clear();
        for e in &self.knowledge {
            require(
                accounts.contains(&e.owner_account_id)
                    && id(&e.knowledge_id)
                    && id(&e.draft_revision_id)
                    && ids.insert(&e.knowledge_id),
                "integrity_mismatch",
            )?;
        }
        for a in &self.accounts {
            require(
                self.roles
                    .iter()
                    .filter(|r| r.owner_account_id == a.account_id)
                    .count()
                    <= 128
                    && self
                        .knowledge
                        .iter()
                        .filter(|r| r.owner_account_id == a.account_id)
                        .count()
                        <= 128
                    && self
                        .migrations
                        .iter()
                        .filter(|r| r.owner_account_id == a.account_id)
                        .count()
                        <= 4
                    && self
                        .activation_history
                        .iter()
                        .filter(|r| r.account_id == a.account_id)
                        .count()
                        <= 100
                    && self
                        .request_results
                        .iter()
                        .filter(|r| r.account_id == a.account_id)
                        .count()
                        <= 1000,
                "storage_capacity_exceeded",
            )?;
            for b in [&a.active, &a.draft] {
                self.validate_refs(&a.account_id, b)?;
            }
            if let Some(r) = &a.legacy_fallback {
                self.validate_ref(&a.account_id, r)?;
            }
        }
        let mut seen = HashSet::new();
        for m in &self.migrations {
            require(
                accounts.contains(&m.owner_account_id)
                    && !m.id.is_empty()
                    && m.id.len() <= 274
                    && hex(&m.source_config_sha256)
                    && hex(&m.seed_content_sha256)
                    && m.completed_generation <= self.generation
                    && seen.insert((&m.owner_account_id, &m.id)),
                "integrity_mismatch",
            )?;
        }
        let mut seen = HashSet::new();
        for h in &self.activation_history {
            require(
                accounts.contains(&h.account_id)
                    && h.generation <= self.generation
                    && h.at <= SAFE_INT
                    && seen.insert((&h.account_id, h.generation)),
                "integrity_mismatch",
            )?;
            h.active.validate()?;
            self.validate_refs(&h.account_id, &h.active)?;
        }
        let mut seen = HashSet::new();
        for r in &self.request_results {
            require(
                accounts.contains(&r.account_id)
                    && [
                        "brain-save-role",
                        "brain-save-knowledge",
                        "brain-copy-role",
                        "brain-archive-role",
                        "brain-save-bindings",
                        "brain-activate",
                        "brain-rollback",
                    ]
                    .contains(&r.operation.as_str())
                    && uuid::Uuid::parse_str(&r.request_id)
                        .is_ok_and(|u| u.to_string() == r.request_id)
                    && hex(&r.request_sha256)
                    && r.at <= SAFE_INT
                    && r.result.generation <= self.generation
                    && seen.insert((&r.account_id, &r.operation, &r.request_id)),
                "integrity_mismatch",
            )?;
            require(
                r.result.role_id.as_deref().is_none_or(id)
                    && r.result.knowledge_id.as_deref().is_none_or(id)
                    && r.result.revision_id.as_deref().is_none_or(id)
                    && r.result.content_sha256.as_deref().is_none_or(hex)
                    && serde_json::to_vec(&r.result)?.len() <= 4096,
                "integrity_mismatch",
            )?;
        }
        Ok(())
    }
    fn validate_ref(&self, account: &str, r: &RoleRef) -> Result<()> {
        require(
            id(&r.role_id)
                && id(&r.revision_id)
                && self
                    .roles
                    .iter()
                    .any(|e| e.owner_account_id == account && e.role_id == r.role_id),
            "integrity_mismatch",
        )
    }
    fn validate_refs(&self, account: &str, b: &BindingSet) -> Result<()> {
        if let Some(r) = &b.default {
            self.validate_ref(account, r)?;
        }
        for o in &b.overrides {
            self.validate_ref(
                account,
                &RoleRef {
                    role_id: o.role_id.clone(),
                    revision_id: o.revision_id.clone(),
                },
            )?;
        }
        Ok(())
    }
}

/// The only unsafe boundary. Every name is a single NUL-free component; no syscall
/// resolves an untrusted path. All returned descriptors are owned by File and closed
/// by Drop, including errors. Child directories and files are always O_NOFOLLOW.
mod unix {
    use std::{
        ffi::{CStr, CString},
        fs::File,
        io,
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::fs::{MetadataExt, PermissionsExt},
        },
    };
    pub(super) struct Dir(pub File);
    fn name(s: &str) -> io::Result<CString> {
        if s.is_empty() || s.contains('/') || s == ".." || s == "." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid component",
            ));
        }
        CString::new(s)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid component"))
    }
    impl Dir {
        pub fn duplicate(&self) -> io::Result<Self> {
            Ok(Self(self.0.try_clone()?))
        }
        pub fn open(&self, n: &str, flags: i32) -> io::Result<File> {
            let n = name(n)?;
            // SAFETY: self owns a live directory FD, n is a terminated single component,
            // and mode is supplied for O_CREAT; successful FD ownership transfers to File.
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    n.as_ptr(),
                    flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: openat returned a fresh, uniquely owned descriptor.
            let file = unsafe { File::from_raw_fd(fd) };
            let m = file.metadata()?;
            if m.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "non-private store entry",
                ));
            }
            if flags & libc::O_DIRECTORY == 0 && !m.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "non-regular store entry",
                ));
            }
            Ok(file)
        }
        pub fn child(&self, n: &str, create: bool) -> io::Result<Self> {
            if create {
                let c = name(n)?;
                // SAFETY: parent FD is live and c is one terminated component; mkdirat does
                // not follow an existing symlink. The subsequent open enforces O_DIRECTORY.
                if unsafe { libc::mkdirat(self.0.as_raw_fd(), c.as_ptr(), 0o700) } < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::AlreadyExists {
                        return Err(e);
                    }
                } else {
                    self.0.sync_all()?;
                }
            }
            Ok(Self(self.open(n, libc::O_RDONLY | libc::O_DIRECTORY)?))
        }
        pub fn unlink(&self, n: &str) -> io::Result<()> {
            let n = name(n)?;
            // SAFETY: live parent FD and one component; never follows the target symlink.
            if unsafe { libc::unlinkat(self.0.as_raw_fd(), n.as_ptr(), 0) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub fn publish(&self, temp: &str, target: &str, replace: bool) -> io::Result<()> {
            let temp = name(temp)?;
            let target = name(target)?;
            // SAFETY: both names are single components in this held directory. renameat
            // replaces a directory entry without following it; linkat(0) never follows it.
            let status = unsafe {
                if replace {
                    libc::renameat(
                        self.0.as_raw_fd(),
                        temp.as_ptr(),
                        self.0.as_raw_fd(),
                        target.as_ptr(),
                    )
                } else {
                    libc::linkat(
                        self.0.as_raw_fd(),
                        temp.as_ptr(),
                        self.0.as_raw_fd(),
                        target.as_ptr(),
                        0,
                    )
                }
            };
            if status < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub fn same(&self, other: &Self) -> io::Result<bool> {
            let a = self.0.metadata()?;
            let b = other.0.metadata()?;
            Ok(a.dev() == b.dev() && a.ino() == b.ino())
        }
        pub fn names(&self) -> io::Result<Vec<String>> {
            // Open a fresh description: readdir offsets must not be shared with the pinned FD.
            let dot = CString::new(".").expect("constant");
            // SAFETY: live FD, constant terminated '.', returned FD is uniquely owned.
            let fd = unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    dot.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: fresh FD from openat, now owned by File until fdopendir succeeds.
            let file = unsafe { File::from_raw_fd(fd) };
            // SAFETY: fdopendir receives a valid directory FD and consumes it only on success.
            let stream = unsafe { libc::fdopendir(file.as_raw_fd()) };
            if stream.is_null() {
                return Err(io::Error::last_os_error());
            }
            let _ = file.into_raw_fd();
            struct Stream(*mut libc::DIR);
            impl Drop for Stream {
                fn drop(&mut self) {
                    // SAFETY: this guard uniquely owns the successful fdopendir stream.
                    unsafe { libc::closedir(self.0) };
                }
            }
            let stream = Stream(stream);
            let mut out = vec![];
            loop {
                // SAFETY: errno pointers are thread-local on each supported Unix platform.
                unsafe {
                    #[cfg(target_os = "macos")]
                    {
                        *libc::__error() = 0;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        *libc::__errno_location() = 0;
                    }
                }
                // SAFETY: stream remains owned/live; each entry is copied before the next call.
                let entry = unsafe { libc::readdir(stream.0) };
                if entry.is_null() {
                    let e = io::Error::last_os_error();
                    if e.raw_os_error() != Some(0) {
                        return Err(e);
                    }
                    break;
                }
                // SAFETY: readdir provides a NUL-terminated d_name valid until the next call.
                let n = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                    .to_str()
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "non UTF-8 store entry")
                    })?;
                if n != "." && n != ".." {
                    out.push(n.to_owned());
                }
            }
            Ok(out)
        }
    }
}
