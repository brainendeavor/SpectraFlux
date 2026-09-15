use std::process::Command;

fn main() {
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let build_timestamp = chrono::Utc::now().to_rfc3339();

    println!("cargo:rustc-env=GIT_HASH={}", git_hash.trim());
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_timestamp);
    println!("cargo:rerun-if-changed=.git/HEAD");
}
