use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub(crate) const VERSION: u32 = 1;
pub(crate) const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Region {
    Sessions,
    Chat,
    Title,
    Search,
}

#[derive(Deserialize)]
pub(crate) struct Response {
    pub protocol_version: u32,
    pub id: u64,
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl Response {
    pub fn into_result(self, expected_id: u64) -> Result<serde_json::Value> {
        if self.protocol_version != VERSION || self.id != expected_id {
            bail!("desktop helper protocol version/request ID mismatch");
        }
        match (self.ok, self.result, self.error) {
            (true, Some(result), None) => Ok(result),
            (false, None, Some(error)) => bail!("desktop helper: {error}"),
            _ => bail!("invalid desktop helper response envelope"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_rejects_stale_ids_versions_and_contradictory_envelopes() {
        for value in [
            serde_json::json!({"protocol_version":2,"id":1,"ok":true,"result":{}}),
            serde_json::json!({"protocol_version":1,"id":2,"ok":true,"result":{}}),
            serde_json::json!({"protocol_version":1,"id":1,"ok":true,"result":{},"error":"failed"}),
            serde_json::json!({"protocol_version":1,"id":1,"ok":false,"result":{}}),
        ] {
            assert!(
                serde_json::from_value::<Response>(value)
                    .unwrap()
                    .into_result(1)
                    .is_err()
            );
        }
    }
}
