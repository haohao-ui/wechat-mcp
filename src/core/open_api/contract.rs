//! External brain open API module.

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimInfo {
    pub claim_token: String,
    pub lease_until: u64,
    pub deadline_at: u64,
}
