use crate::core::config::BotConfig;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// Permission snapshot for the current WeChat account.
pub struct BrainAccess {
    account: String,
    allowed: HashSet<String>,
    unresolved: Vec<String>,
    names: HashMap<(String, String), HashSet<String>>,
}
impl BrainAccess {
    pub fn load(config: &BotConfig, conn: &Connection) -> anyhow::Result<Self> {
        let account = crate::core::wechat_account::effective_account_id(
            &config.state_dir,
            &config.account_id,
        );
        let mut stmt =
            conn.prepare("SELECT contact_id, kind, merged_into FROM contacts WHERE account_id=?")?;
        let contacts: HashMap<String, (String, Option<String>)> = stmt
            .query_map([&account], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<rusqlite::Result<_>>()?;
        // Broken or cyclic merge chains cannot grant access.
        let root_for = |id: &str| -> Option<String> {
            let mut current = id;
            let mut seen = HashSet::new();
            loop {
                if !seen.insert(current) {
                    return None;
                }
                let (_, parent) = contacts.get(current)?;
                match parent {
                    Some(next) => current = next,
                    None => return Some(current.to_string()),
                }
            }
        };
        let roots: HashMap<String, String> = contacts
            .keys()
            .filter_map(|id| root_for(id).map(|root| (id.clone(), root)))
            .collect();
        let mut names: HashMap<(String, String), HashSet<String>> = HashMap::new();
        let mut stmt =
            conn.prepare("SELECT contact_id, name_key FROM contact_names WHERE account_id=?")?;
        let rows = stmt.query_map([&account], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, key) = row?;
            if let (Some((kind, _)), Some(root)) = (contacts.get(&id), roots.get(&id)) {
                names
                    .entry((key, kind.clone()))
                    .or_default()
                    .insert(root.clone());
            }
        }
        let mut unresolved = Vec::new();
        let mut resolve = |entries: &[crate::core::config::AllowlistEntry], ignore_all: bool| {
            let mut resolved = HashSet::new();
            for entry in entries {
                let mut candidates = HashSet::new();
                for name in std::iter::once(&entry.recipient).chain(entry.aliases.iter().flatten())
                {
                    let key = crate::core::history::names::name_key(name);
                    for ((candidate_key, kind), ids) in &names {
                        if candidate_key == &key && (entry.kind.is_empty() || kind == &entry.kind) {
                            candidates.extend(ids.iter().cloned());
                        }
                    }
                }
                if candidates.len() != 1 {
                    unresolved.push(entry.recipient.clone());
                }
                // Ambiguous deny entries must still override uniquely resolved grants.
                if ignore_all || candidates.len() == 1 {
                    resolved.extend(candidates);
                }
            }
            resolved
        };
        let allow = resolve(&config.allowlist, false);
        let ignore = resolve(config.ignorelist.as_deref().unwrap_or(&[]), true);
        let allowed = roots
            .into_iter()
            .filter_map(|(id, root)| {
                (allow.contains(&root) && !ignore.contains(&root)).then_some(id)
            })
            .collect();
        Ok(Self {
            account,
            allowed,
            unresolved,
            names,
        })
    }
    #[cfg(test)]
    pub fn for_test(account: &str, allowed: &[&str], names: &[(&str, &str, &str)]) -> Self {
        let mut resolved: HashMap<(String, String), HashSet<String>> = HashMap::new();
        for (name, kind, id) in names {
            resolved
                .entry((crate::core::history::names::name_key(name), (*kind).into()))
                .or_default()
                .insert((*id).into());
        }
        Self {
            account: account.into(),
            allowed: allowed.iter().map(|id| (*id).into()).collect(),
            unresolved: Vec::new(),
            names: resolved,
        }
    }
    pub fn account_id(&self) -> &str {
        &self.account
    }
    pub fn permit(&self, contact_id: &str) -> bool {
        self.allowed.contains(contact_id)
    }
    pub fn allowed_ids(&self) -> &HashSet<String> {
        &self.allowed
    }
    pub fn contact_for_turn(&self, name: &str, kind: &str) -> Option<String> {
        let candidates = self.names.get(&(
            crate::core::history::names::name_key(name),
            kind.to_string(),
        ))?;
        if candidates.len() != 1 {
            return None;
        }
        candidates
            .iter()
            .next()
            .filter(|id| self.permit(id))
            .cloned()
    }
    pub fn unresolved(&self) -> &[String] {
        &self.unresolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::open_api::compat_tests::Fixture;
    use serde_json::json;

    #[test]
    fn permit_requires_current_account() {
        let f = Fixture::new();
        f.contact("a", "A", "示例群", "group", None);
        f.contact("b", "B", "示例群", "group", None);
        let mut config = f.config(json!([{"recipient":"示例群","kind":"group"}]), json!([]));
        config.account_id = "A".into();
        crate::core::wechat_account::record_current_account(&f.state, "B", 1.0).unwrap();
        let access = BrainAccess::load(&config, &f.conn).unwrap();
        assert_eq!(access.account_id(), "B");
        assert!(access.permit("b"));
        assert!(!access.permit("a"));
    }
    #[test]
    fn ignore_wins_over_allow() {
        let f = Fixture::new();
        f.contact("a", "A", "小美", "private", None);
        let entry = json!([{"recipient":"小美","kind":"private"}]);
        let access = BrainAccess::load(&f.config(entry.clone(), entry), &f.conn).unwrap();
        assert!(!access.permit("a"));
    }
    #[test]
    fn ambiguous_ignore_name_ignores_all_candidates() {
        let f = Fixture::new();
        f.contact("a", "A", "小美", "private", None);
        f.contact("b", "A", "小美", "private", None);
        f.conn.execute(
            "INSERT INTO contact_names(contact_id,account_id,raw_name,name_key,source,first_seen_at,last_seen_at) VALUES('a','A','美美',?,'alias',1,1)",
            [crate::core::history::names::name_key("美美")],
        ).unwrap();
        let config = f.config(
            json!([{"recipient":"美美","kind":"private"}]),
            json!([{"recipient":"小美","kind":"private"}]),
        );
        let access = BrainAccess::load(&config, &f.conn).unwrap();
        assert!(!access.permit("a"));
        assert!(!access.permit("b"));
        assert_eq!(access.unresolved(), &["小美"]);
    }

    #[test]
    fn ambiguous_name_is_unresolved() {
        let f = Fixture::new();
        for id in ["a", "b"] {
            f.contact(id, "A", "小美", "private", None);
        }
        let access = BrainAccess::load(
            &f.config(json!([{"recipient":"小美","kind":"private"}]), json!([])),
            &f.conn,
        )
        .unwrap();
        assert!(!access.permit("a"));
        assert!(!access.permit("b"));
        assert_eq!(access.unresolved(), &["小美"]);
    }
    #[test]
    fn permit_follows_merge_family() {
        let f = Fixture::new();
        f.contact("r", "A", "根", "private", None);
        f.contact("c", "A", "子", "private", Some("r"));
        f.contact("r2", "A", "忽略根", "private", None);
        f.contact("c2", "A", "忽略子", "private", Some("r2"));
        for name in ["根", "子"] {
            let access = BrainAccess::load(&f.config(json!([{"recipient":name,"kind":"private"},{"recipient":"忽略子","kind":"private"}]),json!([{"recipient":"忽略根","kind":"private"}])), &f.conn).unwrap();
            assert!(access.permit("r"));
            assert!(access.permit("c"));
            assert!(!access.permit("r2"));
            assert!(!access.permit("c2"));
            assert_eq!(access.contact_for_turn("子", "private"), Some("r".into()));
        }
    }
    #[test]
    fn contact_for_turn_matches_kind() {
        let f = Fixture::new();
        f.contact("p", "A", "同名", "private", None);
        f.contact("g", "A", "同名", "group", None);
        let access = BrainAccess::load(
            &f.config(
                json!([{"recipient":"同名","kind":"private"},{"recipient":"同名","kind":"group"}]),
                json!([]),
            ),
            &f.conn,
        )
        .unwrap();
        assert_eq!(access.contact_for_turn("同名", "private"), Some("p".into()));
        assert_eq!(access.contact_for_turn("同名", "group"), Some("g".into()));
        assert_eq!(access.contact_for_turn("同名", "other"), None);
    }
}
