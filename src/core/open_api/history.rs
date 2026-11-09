use super::access::BrainAccess;
use crate::core::history::{
    mcp_tools,
    query::{HistoryQuery, QueryError},
};
use serde_json::Value;

/// History tools exposed to an external brain through a permission snapshot.
pub struct RestrictedHistory {
    query: HistoryQuery,
    access: BrainAccess,
}
impl RestrictedHistory {
    pub fn new(query: HistoryQuery, access: BrainAccess) -> Self {
        Self {
            query: query.with_allowlist(access.allowed_ids().clone()),
            access,
        }
    }
    pub fn check_contact(&self, contact_id: &str) -> Result<(), QueryError> {
        if self.access.permit(contact_id) {
            Ok(())
        } else {
            Err(QueryError::not_found("unknown contact"))
        }
    }

    pub fn call_tool(&self, name: &str, args: &Value) -> Result<Value, QueryError> {
        // Reuse the published schema so the wrapper and ops accept the same arguments.
        let schema = Self::tools()
            .into_iter()
            .find(|tool| tool["name"] == name)
            .ok_or_else(|| QueryError::invalid_argument("unknown tool"))?;
        let object = args
            .as_object()
            .ok_or_else(|| QueryError::invalid_argument("invalid arguments"))?;
        let properties = schema["inputSchema"]["properties"].as_object().unwrap();
        for required in schema["inputSchema"]["required"].as_array().unwrap() {
            if !object.contains_key(required.as_str().unwrap()) {
                return Err(QueryError::invalid_argument("invalid arguments"));
            }
        }
        for (key, value) in object {
            let property = properties
                .get(key)
                .ok_or_else(|| QueryError::invalid_argument("invalid arguments"))?;
            match property["type"].as_str() {
                Some("string") if !value.is_string() => {
                    return Err(QueryError::invalid_argument(format!(
                        "{key} must be string"
                    )));
                }
                Some("integer") if !value.is_i64() && !value.is_u64() => {
                    return Err(QueryError::invalid_argument(format!(
                        "{key} must be integer"
                    )));
                }
                _ => {}
            }
        }
        if let Some(id) = args["contact_id"].as_str() {
            self.check_contact(id)?;
        }
        if name == "history_search_messages" {
            let contacts = if let Some(id) = args["contact_id"].as_str() {
                std::collections::HashSet::from([id.to_string()])
            } else {
                self.access.allowed_ids().clone()
            };
            return self.query.search_messages_in(
                args["q"].as_str().unwrap(),
                &contacts,
                args["since"].as_i64(),
                args["until"].as_i64(),
                args["cursor"].as_str(),
                args["limit"].as_i64(),
            );
        }
        if name == "history_get_contact" && args["contact_id"].is_null() {
            let name = args["name"].as_str().ok_or_else(|| {
                QueryError::invalid_argument("give exactly one of contact_id or name")
            })?;
            return self.contact_by_name(name);
        }
        if name == "history_get_message" {
            self.check_message(args["message_id"].as_str().unwrap())?;
        }
        if name == "history_get_media" {
            let owners = self
                .query
                .media_owners(args["media_id"].as_str().unwrap())?;
            if !owners.iter().any(|(_, contact, account)| {
                self.access.permit(contact) && account == self.access.account_id()
            }) {
                return Err(QueryError::not_found("unknown media"));
            }
        }
        let result = mcp_tools::call_tool(&self.query, name, args)?;
        // The ops serializer omits account_id; verify it against the source row.
        if let Some(items) = result["items"].as_array() {
            for item in items {
                if let Some(id) = item["message_id"].as_str() {
                    self.check_message(id)?;
                }
            }
        }
        Ok(result)
    }

    fn contact_by_name(&self, name: &str) -> Result<Value, QueryError> {
        let key = crate::core::history::names::name_key(name);
        let mut stmt = self.query.raw_connection().prepare(
            "SELECT contact_id FROM contact_names WHERE account_id=? AND name_key=? AND source IN ('config','alias','addressbook')")
            .map_err(|e| QueryError::new("db_error",e.to_string()))?;
        let rows = stmt
            .query_map([self.access.account_id(), &key], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        let mut contacts = std::collections::HashMap::new();
        for row in rows {
            let id = row.map_err(|e| QueryError::new("db_error", e.to_string()))?;
            if self.access.permit(&id) {
                let contact = self.query.get_contact(Some(&id), None)?;
                contacts.insert(contact["contact_id"].as_str().unwrap().to_string(), contact);
            }
        }
        if contacts.len() != 1 {
            return Err(QueryError::not_found("unknown contact"));
        }
        Ok(contacts.into_values().next().unwrap())
    }

    fn check_message(&self, id: &str) -> Result<(), QueryError> {
        use rusqlite::OptionalExtension;
        let owner = self
            .query
            .raw_connection()
            .query_row(
                "SELECT contact_id, account_id FROM messages WHERE message_id=?",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| QueryError::new("db_error", e.to_string()))?;
        match owner {
            Some((contact, account))
                if self.access.permit(&contact) && account == self.access.account_id() =>
            {
                Ok(())
            }
            _ => Err(QueryError::not_found("unknown message")),
        }
    }
    pub fn tools() -> Vec<Value> {
        mcp_tools::tools()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::open_api::compat_tests::Fixture;
    use serde_json::json;

    fn setup() -> Fixture {
        let f = Fixture::new();
        f.contact("allowed", "A", "允许", "private", None);
        f.contact("forbidden", "A", "禁止", "private", None);
        f.contact("ignored", "A", "忽略", "private", None);
        f.contact("other", "B", "允许", "private", None);
        f
    }
    fn restricted(f: &Fixture) -> RestrictedHistory {
        let config = f.config(
            json!([{"recipient":"允许","kind":"private"},{"recipient":"忽略","kind":"private"}]),
            json!([{"recipient":"忽略","kind":"private"}]),
        );
        RestrictedHistory::new(f.query(), BrainAccess::load(&config, &f.conn).unwrap())
    }
    fn denied(result: Result<Value, QueryError>) {
        assert_eq!(result.unwrap_err().code, "not_found");
    }

    #[test]
    fn restricted_get_history_denies_forbidden_contact() {
        let f = setup();
        let h = restricted(&f);
        for tool in [
            "history_get_history",
            "history_get_recent_messages",
            "history_get_contact",
        ] {
            denied(h.call_tool(tool, &json!({"contact_id":"forbidden"})));
        }
        denied(h.call_tool(
            "history_search_messages",
            &json!({"contact_id":"forbidden","q":"needle"}),
        ));
    }
    #[test]
    fn restricted_get_message_denies_other_account() {
        let f = setup();
        f.message("other-message", "other", "B", 1, None);
        f.message("mismatched-account", "allowed", "B", 1, None);
        f.message("good", "allowed", "A", 2, None);
        let h = restricted(&f);
        for id in ["other-message", "mismatched-account"] {
            denied(h.call_tool("history_get_message", &json!({"message_id":id})));
        }
        assert_eq!(
            h.call_tool("history_get_message", &json!({"message_id":"good"}))
                .unwrap()["message_id"],
            "good"
        );
    }
    #[test]
    fn media_without_message_denied() {
        let f = setup();
        f.media("orphan");
        denied(restricted(&f).call_tool("history_get_media", &json!({"media_id":"orphan"})));
    }
    #[test]
    fn media_shared_between_allowed_and_forbidden() {
        let f = setup();
        f.media("shared");
        f.message("bad", "forbidden", "A", 1, Some("shared"));
        f.message("good", "allowed", "A", 1, Some("shared"));
        assert_eq!(
            restricted(&f)
                .call_tool("history_get_media", &json!({"media_id":"shared"}))
                .unwrap()["base64"],
            "YWJj"
        );
        let owners = f.query().media_owners("shared").unwrap();
        assert_eq!(owners.len(), 2);
    }
    #[test]
    fn media_of_ignored_contact_denied() {
        let f = setup();
        f.media("ignored-media");
        f.message("ignored-message", "ignored", "A", 1, Some("ignored-media"));
        f.message("wrong-account", "allowed", "B", 1, Some("ignored-media"));
        denied(restricted(&f).call_tool("history_get_media", &json!({"media_id":"ignored-media"})));
    }
    #[test]
    fn search_filters_before_paging() {
        let f = setup();
        f.message("old-allowed", "allowed", "A", 1, None);
        f.message("new-allowed", "allowed", "A", 2, None);
        for seq in 1..=20 {
            f.message(&format!("denied-{seq}"), "forbidden", "A", seq, None);
        }
        let h = restricted(&f);
        let page = h
            .call_tool("history_search_messages", &json!({"q":"needle","limit":5}))
            .unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["items"][0]["message_id"], "new-allowed");
        assert!(page["next_cursor"].is_null());
        let first = h
            .call_tool("history_search_messages", &json!({"q":"needle","limit":1}))
            .unwrap();
        assert!(first["next_cursor"].is_string());
        let second = h
            .call_tool(
                "history_search_messages",
                &json!({"q":"needle","limit":1,"cursor":first["next_cursor"]}),
            )
            .unwrap();
        assert_eq!(second["items"][0]["message_id"], "old-allowed");
        assert!(second["next_cursor"].is_null());
        let specific = h
            .call_tool(
                "history_search_messages",
                &json!({"q":"ne","contact_id":"allowed","limit":5}),
            )
            .unwrap();
        assert_eq!(specific["items"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn restricted_contacts_resolve_names_in_current_account() {
        let f = setup();
        let h = restricted(&f);
        assert_eq!(
            h.call_tool("history_get_contact", &json!({"name":"允许"}))
                .unwrap()["contact_id"],
            "allowed"
        );
        denied(h.call_tool("history_get_contact", &json!({"name":"禁止"})));
        let page = h
            .call_tool("history_list_contacts", &json!({"limit":1}))
            .unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["contact_id"], "allowed");
        assert!(page["next_cursor"].is_null());
    }

    #[test]
    fn resume_contact_must_be_permitted() {
        let f = setup();
        let h = restricted(&f);
        assert_eq!(h.check_contact("forbidden").unwrap_err().code, "not_found");
        assert!(h.check_contact("allowed").is_ok());
    }
    #[test]
    fn restricted_tools_list_has_seven_history_tools() {
        let tools = RestrictedHistory::tools();
        assert_eq!(tools.len(), 7);
        assert_eq!(tools, mcp_tools::tools());
    }
}
