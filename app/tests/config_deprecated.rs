use rustkvm::config::Config;
use rustkvm::config::persistence::ConfigPersistence;

async fn write_config(dir: &std::path::Path, extra: &str) -> ConfigPersistence {
    let path = dir.join("config.toml");
    let base = toml::to_string_pretty(&Config::default()).expect("serialize defaults");
    std::fs::write(&path, format!("{extra}\n{base}")).expect("write config");
    ConfigPersistence::with_path(path)
}

#[tokio::test]
async fn deprecated_ai_enabled_is_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = write_config(dir.path(), "ai_enabled = false").await;
    let loaded = persistence.load().await.expect("deprecated key must not fail load");
    assert_eq!(loaded.audio_enabled, Config::default().audio_enabled);

    persistence.save(&loaded).await.expect("save");
    let rewritten = std::fs::read_to_string(persistence.path()).expect("read back");
    assert!(!rewritten.contains("ai_enabled"), "deprecated key survived save");
}

#[tokio::test]
async fn unknown_fields_are_still_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let persistence = write_config(dir.path(), "bogus_field = 1").await;
    let err = persistence.load().await.expect_err("typo must still be rejected");
    assert!(format!("{err:?}").contains("bogus_field"), "{err:?}");
}
