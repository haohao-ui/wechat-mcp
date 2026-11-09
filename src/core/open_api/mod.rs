// These APIs are consumed by the later open-api transport and turn tasks.
#![allow(dead_code)]
pub mod access;
pub mod contract;
pub mod history;
pub mod http;
pub mod ledger;
pub mod session;

#[derive(Debug, Clone, PartialEq)]
pub enum OpenApiError {
    TurnNotFound,
    TurnExpired,
    TurnRevoked,
    AlreadyAnswered { action: String },
    InvalidClaimToken,
    NotFound,
    InvalidText,
    InvalidArguments(String),
    RateLimited,
    BrainModeNotExternal,
    Internal(String),
}
impl OpenApiError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TurnNotFound => "turn_not_found",
            Self::TurnExpired => "turn_expired",
            Self::TurnRevoked => "turn_revoked",
            Self::AlreadyAnswered { .. } => "already_answered",
            Self::InvalidClaimToken => "invalid_claim_token",
            Self::NotFound => "not_found",
            Self::InvalidText => "invalid_text",
            Self::InvalidArguments(_) => "invalid_arguments",
            Self::RateLimited => "rate_limited",
            Self::BrainModeNotExternal => "brain_mode_not_external",
            Self::Internal(_) => "internal",
        }
    }
}
#[cfg(test)]
mod compat_tests;
