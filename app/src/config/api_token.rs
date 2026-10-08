use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const TOKEN_PREFIX: &str = "rkvm_";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenState {
    pub enabled: bool,
    pub created_at: Option<String>,
}

pub fn generate_token() -> String {
    format!("{TOKEN_PREFIX}{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn verify_token(token: &str, stored_hash: &str) -> bool {
    let Ok(expected) = hex::decode(stored_hash) else {
        return false;
    };
    if token.is_empty() || expected.len() != 32 {
        return false;
    }
    let actual = Sha256::digest(token.as_bytes());
    // Constant-time so response timing does not leak how many hash bytes matched.
    actual.iter().zip(expected.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

pub fn bearer_token(authorization: &str) -> Option<&str> {
    let (scheme, token) = authorization.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}
