use std::fs;

use rustkvm::update::{
    ASSET_NAME, BootDecision, GithubAsset, GithubRelease, InstallPaths, MAX_BOOT_ATTEMPTS,
    RELEASE_DOWNLOAD_PREFIX, confirm, install_staged, is_official_download, on_boot, parse_sha256,
    select_release, stage_binary, verify_aarch64_elf,
};
use sha2::{Digest, Sha256};

fn release(tag: &str, prerelease: bool, with_sha: bool) -> GithubRelease {
    let mut assets = vec![GithubAsset {
        name: ASSET_NAME.to_string(),
        browser_download_url: format!(
            "https://github.com/Rust-KVM/rustkvm/releases/download/{tag}/{ASSET_NAME}"
        ),
    }];
    if with_sha {
        assets.push(GithubAsset {
            name: format!("{ASSET_NAME}.sha256"),
            browser_download_url: format!(
                "https://github.com/Rust-KVM/rustkvm/releases/download/{tag}/{ASSET_NAME}.sha256"
            ),
        });
    }
    GithubRelease { tag_name: tag.to_string(), prerelease, draft: false, assets }
}

fn fake_aarch64_elf(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len.max(64)];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[18..20].copy_from_slice(&0xB7u16.to_le_bytes());
    bytes
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn install_dir() -> (tempfile::TempDir, InstallPaths) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = InstallPaths::new(dir.path().join("rustkvm_app"));
    fs::write(paths.current(), b"old").expect("seed current binary");
    (dir, paths)
}

#[test]
fn select_release_picks_newest_eligible() {
    let releases = vec![
        release("v0.2.0", false, true),
        release("v0.4.0-rc.1", true, true),
        release("v0.3.0", false, true),
        release("v0.9.0", false, false),
        GithubRelease { draft: true, ..release("v1.0.0", false, true) },
    ];
    let stable = select_release(&releases, "0.1.0", false).expect("stable update");
    assert_eq!(stable.version, "0.3.0");
    assert!(stable.sha256_url.ends_with("/v0.3.0/rustkvm_app-aarch64-unknown-linux-gnu.sha256"));

    let pre = select_release(&releases, "0.1.0", true).expect("pre-release update");
    assert_eq!(pre.version, "0.4.0-rc.1");

    assert!(select_release(&releases, "0.3.0", false).is_none());
    assert!(select_release(&releases, "not-semver", false).is_none());
}

#[test]
fn only_official_release_assets_are_accepted() {
    assert!(is_official_download(&format!("{RELEASE_DOWNLOAD_PREFIX}v1.0.0/{ASSET_NAME}")));
    assert!(!is_official_download("https://evil.example/rustkvm/releases/download/v1/x"));
    assert!(!is_official_download(&format!("{RELEASE_DOWNLOAD_PREFIX}../../other/repo/x")));
    assert!(!is_official_download("http://github.com/Rust-KVM/rustkvm/releases/download/v1/x"));

    let mut foreign = release("v9.0.0", false, true);
    foreign.assets[0].browser_download_url = "https://evil.example/bin".to_string();
    assert!(select_release(&[foreign], "0.1.0", false).is_none());
}

#[test]
fn sha256_file_formats() {
    let digest = hex::encode([0xabu8; 32]);
    assert_eq!(parse_sha256(&digest).expect("bare"), [0xab; 32]);
    assert_eq!(parse_sha256(&format!("{digest}  {ASSET_NAME}\n")).expect("sha256sum"), [0xab; 32]);
    assert!(parse_sha256("").is_err());
    assert!(parse_sha256("abcd").is_err());
    assert!(parse_sha256("zz").is_err());
}

#[test]
fn elf_check_rejects_wrong_architecture() {
    assert!(verify_aarch64_elf(&fake_aarch64_elf(64)).is_ok());
    let mut x86 = fake_aarch64_elf(64);
    x86[18..20].copy_from_slice(&0x3Eu16.to_le_bytes());
    assert!(verify_aarch64_elf(&x86).is_err());
    assert!(verify_aarch64_elf(b"#!/bin/sh\n").is_err());
    let mut elf32 = fake_aarch64_elf(64);
    elf32[4] = 1;
    assert!(verify_aarch64_elf(&elf32).is_err());
}

#[test]
fn staging_rejects_bad_checksum_and_keeps_current() {
    let (_dir, paths) = install_dir();
    let bin = fake_aarch64_elf(128);
    assert!(stage_binary(&paths, &bin, &[0u8; 32]).is_err());
    assert!(!paths.staged().exists());
    assert!(install_staged(&paths, "0.1.0", "0.2.0").is_err());
    assert_eq!(fs::read(paths.current()).expect("current"), b"old");
}

#[test]
fn install_then_confirm() {
    let (_dir, paths) = install_dir();
    let bin = fake_aarch64_elf(128);
    stage_binary(&paths, &bin, &sha(&bin)).expect("stage");
    install_staged(&paths, "0.1.0", "0.2.0").expect("install");

    assert_eq!(fs::read(paths.current()).expect("current"), bin);
    assert_eq!(fs::read(paths.previous()).expect("previous"), b"old");
    assert!(!paths.staged().exists());

    match on_boot(&paths).expect("boot") {
        BootDecision::Probation(p) => {
            assert_eq!(
                (p.from_version.as_str(), p.to_version.as_str(), p.attempts),
                ("0.1.0", "0.2.0", 1)
            );
        }
        other => panic!("expected probation, got {other:?}"),
    }
    confirm(&paths).expect("confirm");
    assert_eq!(on_boot(&paths).expect("boot"), BootDecision::Normal);
    confirm(&paths).expect("confirm is idempotent");
}

#[test]
fn unconfirmed_update_rolls_back_after_max_attempts() {
    let (_dir, paths) = install_dir();
    let bin = fake_aarch64_elf(128);
    stage_binary(&paths, &bin, &sha(&bin)).expect("stage");
    install_staged(&paths, "0.1.0", "0.2.0").expect("install");

    for attempt in 1..=MAX_BOOT_ATTEMPTS {
        assert!(
            matches!(on_boot(&paths).expect("boot"), BootDecision::Probation(p) if p.attempts == attempt)
        );
    }
    assert!(matches!(on_boot(&paths).expect("boot"), BootDecision::RolledBack(_)));
    assert_eq!(fs::read(paths.current()).expect("current"), b"old");
    assert!(!paths.marker().exists());
    assert_eq!(on_boot(&paths).expect("boot"), BootDecision::Normal);
}
