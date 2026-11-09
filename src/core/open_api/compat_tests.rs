use crate::core::history::{mcp_tools, query::HistoryQuery, store::HistoryStore};
use serde_json::{Value, json};
use std::{collections::HashSet, fs, path::PathBuf};

pub(super) struct Fixture {
    pub state: PathBuf,
    pub conn: rusqlite::Connection,
}
impl Fixture {
    pub fn new() -> Self {
        let state = std::env::temp_dir().join(format!("open-api-test-{}", uuid::Uuid::new_v4()));
        drop(HistoryStore::open(&state).unwrap());
        let conn = rusqlite::Connection::open(state.join("history/history.sqlite3")).unwrap();
        Self { state, conn }
    }
    pub fn contact(&self, id: &str, account: &str, name: &str, kind: &str, root: Option<&str>) {
        self.conn.execute("INSERT INTO contacts(contact_id,account_id,kind,display_name,merged_into,created_at,updated_at) VALUES(?,?,?,?,?,1,1)", rusqlite::params![id,account,kind,name,root]).unwrap();
        self.conn.execute("INSERT INTO contact_names(contact_id,account_id,raw_name,name_key,source,first_seen_at,last_seen_at) VALUES(?,?,?,?,'config',1,1)", rusqlite::params![id,account,name,crate::core::history::names::name_key(name)]).unwrap();
    }
    pub fn message(&self, id: &str, contact: &str, account: &str, seq: i64, media: Option<&str>) {
        self.conn.execute("INSERT INTO messages(message_id,contact_id,account_id,seq,direction,sender,kind,text,observed_at,capture,media_id) VALUES(?,?,?,?,'inbound','customer','text','needle',1,'live',?)", rusqlite::params![id,contact,account,seq,media]).unwrap();
    }
    pub fn media(&self, id: &str) {
        let dir = self.state.join("history/media");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(id), b"abc").unwrap();
        self.conn.execute("INSERT INTO media(media_id,kind,filename,extension,size,source,path,created_at) VALUES(?,'file','a.txt','txt',3,'test',?,1)", rusqlite::params![id,id]).unwrap();
    }
    pub fn config(&self, allow: Value, ignore: Value) -> crate::core::config::BotConfig {
        let path = self.state.join("config.json");
        fs::write(&path,json!({"schema_version":1,"host":"local","account_id":"A","state_dir":self.state,"allowlist":[{"recipient":"fixture","kind":"private"}]}).to_string()).unwrap();
        let mut config = crate::core::config::BotConfig::load(path).unwrap();
        config.allowlist = serde_json::from_value(allow).unwrap();
        config.ignorelist = Some(serde_json::from_value(ignore).unwrap());
        config
    }
    pub fn query(&self) -> HistoryQuery {
        HistoryQuery::open(self.state.join("history/history.sqlite3"), None).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.state).unwrap();
    }
}

#[test]
fn ops_tools_list_unchanged() {
    assert_eq!(
        serde_json::to_string_pretty(&crate::tools::list_tools_with_legacy(true)).unwrap() + "\n",
        include_str!("../../../tests/fixtures/ops_tools_list.json")
    );
}

fn message() -> Value {
    json!({"message_id":"m","contact_id":"denied","seq":1,"direction":"inbound","sender":"customer","kind":"text","text":"needle","observed_at":1,"capture":"live","send":null,"media":{"media_id":"md","kind":"file","filename":"a.txt","source":"test","understanding":null}})
}

#[test]
fn ops_history_behavior_unchanged() {
    let f = Fixture::new();
    f.contact("denied", "other", "禁止", "private", None);
    f.media("md");
    f.message("m", "denied", "other", 1, Some("md"));
    let q = f.query().with_allowlist(HashSet::new());
    assert_eq!(
        mcp_tools::call_tool(&q, "history_list_contacts", &json!({})).unwrap(),
        json!({"items":[],"next_cursor":null})
    );
    assert_eq!(
        mcp_tools::call_tool(&q, "history_get_contact", &json!({"contact_id":"denied"}))
            .unwrap_err()
            .code,
        "not_found"
    );
    let page = json!({"items":[message()],"next_cursor":null});
    let mut history = page.clone();
    history["tail_cursor"] = json!("WzEsImFzYyJd");
    assert_eq!(
        mcp_tools::call_tool(&q, "history_get_history", &json!({"contact_id":"denied"})).unwrap(),
        history
    );
    assert_eq!(
        mcp_tools::call_tool(
            &q,
            "history_get_recent_messages",
            &json!({"contact_id":"denied"})
        )
        .unwrap(),
        json!({"items":[message()]})
    );
    let mut search = page.clone();
    search["items"][0]["context_ids"] = json!([]);
    assert_eq!(
        mcp_tools::call_tool(&q, "history_search_messages", &json!({"q":"needle"})).unwrap(),
        search
    );
    let mut detailed = message();
    detailed["meta"] = json!({});
    assert_eq!(
        mcp_tools::call_tool(&q, "history_get_message", &json!({"message_id":"m"})).unwrap(),
        detailed
    );
    assert_eq!(
        mcp_tools::call_tool(&q, "history_get_media", &json!({"media_id":"md"})).unwrap(),
        json!({"media":{"media_id":"md","kind":"file","filename":"a.txt","extension":"txt","size":3,"source":"test","understanding":null,"understood_at":null},"base64":"YWJj"})
    );
}
