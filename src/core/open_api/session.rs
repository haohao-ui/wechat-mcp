//! Single external-brain session, independent of HTTP connection lifetimes.
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub struct Session {
    pub id: String,
    pub token: String,
    pub last_seen: Instant,
    pub ended: watch::Sender<bool>,
    pub api_version: u32,
    pub capabilities: Vec<String>,
}

pub struct Sessions {
    pub active: Option<Session>,
    pub lease: Duration,
}
impl Default for Sessions {
    fn default() -> Self {
        Self {
            active: None,
            lease: Duration::from_secs(90),
        }
    }
}
impl Sessions {
    pub fn take_expired(&mut self, token: &str) -> Option<Session> {
        if self
            .active
            .as_ref()
            .is_some_and(|s| s.token != token || s.last_seen.elapsed() >= self.lease)
        {
            self.end()
        } else {
            None
        }
    }
    pub fn end(&mut self) -> Option<Session> {
        let session = self.active.take()?;
        let _ = session.ended.send(true);
        Some(session)
    }
    pub fn initialize(&mut self, token: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let (ended, _) = watch::channel(false);
        self.active = Some(Session {
            id: id.clone(),
            token,
            last_seen: Instant::now(),
            ended,
            api_version: 1,
            capabilities: Vec::new(),
        });
        id
    }
}
