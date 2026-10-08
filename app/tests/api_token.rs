use rustkvm::config::Config;
use rustkvm::config::api_token::{
    TOKEN_PREFIX, bearer_token, generate_token, hash_token, verify_token,
};

#[test]
fn generated_tokens_are_prefixed_unique_and_long() {
    let a = generate_token();
    let b = generate_token();
    assert!(a.starts_with(TOKEN_PREFIX));
    assert_eq!(a.len(), TOKEN_PREFIX.len() + 64);
    assert_ne!(a, b);
}

#[test]
fn hash_is_sha256_hex() {
    assert_eq!(
        hash_token("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn verify_accepts_only_the_matching_token() {
    let token = generate_token();
    let stored = hash_token(&token);
    assert!(verify_token(&token, &stored));
    assert!(!verify_token(&generate_token(), &stored));
    assert!(!verify_token("", &hash_token("")));
    assert!(!verify_token(&token, "not-hex"));
    assert!(!verify_token(&token, "abcd"));
}

#[test]
fn bearer_header_parsing() {
    assert_eq!(bearer_token("Bearer rkvm_x"), Some("rkvm_x"));
    assert_eq!(bearer_token("bearer  rkvm_x "), Some("rkvm_x"));
    assert_eq!(bearer_token("Basic dXNlcjpwYXNz"), None);
    assert_eq!(bearer_token("Bearer "), None);
    assert_eq!(bearer_token("Bearer"), None);
}

#[test]
fn config_without_token_fields_still_deserializes() {
    let mut value = serde_json::to_value(Config::default()).expect("serialize default config");
    let obj = value.as_object_mut().expect("config is an object");
    assert!(!obj.contains_key("api_token_sha256"));
    obj.remove("api_token_created_at");
    let config: Config = serde_json::from_value(value).expect("deserialize legacy config");
    assert!(config.api_token_sha256.is_none());
}
