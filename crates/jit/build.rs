use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-env-changed=JIT_BUILD_GIT_HASH");
    println!("cargo:rerun-if-env-changed=JIT_BUILD_GIT_SHORT_HASH");
    println!("cargo:rerun-if-env-changed=JIT_BUILD_GIT_DIRTY");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    if let Some(head_ref) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed=../../.git/{}", head_ref);
    }

    let git_hash =
        env_override("JIT_BUILD_GIT_HASH").or_else(|| git_output(&["rev-parse", "HEAD"]));
    let git_short_hash = env_override("JIT_BUILD_GIT_SHORT_HASH")
        .or_else(|| git_output(&["rev-parse", "--short=8", "HEAD"]));
    let git_dirty = env_override("JIT_BUILD_GIT_DIRTY")
        .unwrap_or_else(|| git_dirty().unwrap_or_else(|| "unknown".to_string()));

    println!(
        "cargo:rustc-env=JIT_GIT_HASH={}",
        git_hash.unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=JIT_GIT_SHORT_HASH={}",
        git_short_hash.unwrap_or_else(|| "unknown".to_string())
    );
    println!("cargo:rustc-env=JIT_GIT_DIRTY={}", git_dirty);
    println!(
        "cargo:rustc-env=JIT_BUILD_PROFILE={}",
        env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=JIT_BUILD_TARGET={}",
        env::var("TARGET").unwrap_or_else(|_| "unknown".to_string())
    );
    println!("cargo:rustc-env=JIT_BUILD_TIMESTAMP={}", build_timestamp());
}

fn env_override(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn git_output(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| stdout.trim().to_string())
        .filter(|stdout| !stdout.is_empty())
}

fn git_dirty() -> Option<String> {
    Command::new("git")
        .args(["status", "--short", "--untracked-files=normal"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| (!stdout.trim().is_empty()).to_string())
}

fn build_timestamp() -> String {
    env_override("SOURCE_DATE_EPOCH").unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    })
}
