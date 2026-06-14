use std::process::Command;

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-env-changed=GIT_REVISION");
    println!("cargo:rerun-if-env-changed=GIT_BRANCH");
    println!("cargo:rerun-if-env-changed=BUILD_DATE");

    let rev = std::env::var("GIT_REVISION")
        .ok()
        .or_else(|| git_output(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=GIT_REVISION={rev}");

    let branch = std::env::var("GIT_BRANCH")
        .ok()
        .or_else(|| git_output(&["rev-parse", "--abbrev-ref", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=GIT_BRANCH={branch}");

    let date = std::env::var("BUILD_DATE")
        .ok()
        .or_else(|| {
            Command::new("date")
                .arg("-u")
                .arg("+%Y-%m-%dT%H:%M:%SZ")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=BUILD_DATE={date}");

    let rustc = std::env::var("RUSTC_VERSION")
        .ok()
        .or_else(|| {
            Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=RUSTC_VERSION={rustc}");
}
