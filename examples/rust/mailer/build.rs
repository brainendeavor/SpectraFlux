use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wit/fluxcell.wit");

    // 1. Derive git commit hash
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "00000000".to_string());

    // 2. Derive semver version from git describe or fallback to CARGO_PKG_VERSION
    let git_tag = Command::new("git")
        .args(["describe", "--tags", "--always"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let build_time = chrono::Utc::now().to_rfc3339();

    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=FLUXCELL_VERSION={}", git_tag);
    println!("cargo:rustc-env=BUILD_TIME={}", build_time);
}
