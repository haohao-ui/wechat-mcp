pub mod accounts;
pub mod adapters;
pub mod article;
pub mod brain;
pub mod clipboard;
pub mod core;
pub mod crm;
pub mod desktop;
pub mod domain;
pub mod gateway;
pub mod interaction;
pub mod listeners;
pub mod mcp;
pub mod messaging;
pub mod resource_lock;
#[cfg(test)]
pub mod service_tests;
pub mod service_tools;
pub mod storage;
pub mod tools;
pub mod updater;
pub mod wechat;

pub const SUPPORTED_PROTOCOL_VERSION: &str = "2025-11-25";
pub const FALLBACK_PROTOCOL_VERSION: &str = "2024-11-05";

fn is_valid_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits_valid = bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit());
    if !digits_valid {
        return false;
    }
    let month: u32 = match s[5..7].parse() {
        Ok(m) => m,
        Err(_) => return false,
    };
    let day: u32 = match s[8..10].parse() {
        Ok(d) => d,
        Err(_) => return false,
    };
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

pub fn negotiate_protocol_version(client_version: Option<&str>) -> (&'static str, bool) {
    match client_version {
        Some(v) if is_valid_date(v) && v >= SUPPORTED_PROTOCOL_VERSION => {
            (SUPPORTED_PROTOCOL_VERSION, true)
        }
        _ => (FALLBACK_PROTOCOL_VERSION, false),
    }
}
