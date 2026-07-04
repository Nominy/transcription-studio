use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=STUDIO_BUILD_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    print_git_rerun_hints();

    let commit = env::var("STUDIO_BUILD_GIT_COMMIT")
        .ok()
        .and_then(normalize_commit)
        .or_else(|| env::var("GITHUB_SHA").ok().and_then(normalize_commit))
        .or_else(git_head_commit)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=STUDIO_GIT_COMMIT={commit}");
}

fn normalize_commit(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(12).collect())
}

fn git_head_commit() -> Option<String> {
    let workspace_root = workspace_root()?;
    let output = Command::new("git")
        .args(["-C"])
        .arg(workspace_root)
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    normalize_commit(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn print_git_rerun_hints() {
    let Some(workspace_root) = workspace_root() else {
        return;
    };
    let git_path = workspace_root.join(".git");
    let git_dir = resolve_git_dir(&git_path).unwrap_or(git_path);
    let head_path = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head_path.display());

    let Ok(head) = fs::read_to_string(&head_path) else {
        return;
    };
    if let Some(ref_name) = head.trim().strip_prefix("ref: ") {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(ref_name).display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );
}

fn workspace_root() -> Option<PathBuf> {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR")?)
        .parent()?
        .parent()
        .map(Path::to_path_buf)
}

fn resolve_git_dir(git_path: &Path) -> Option<PathBuf> {
    if git_path.is_dir() {
        return Some(git_path.to_path_buf());
    }
    let content = fs::read_to_string(git_path).ok()?;
    let relative = content.trim().strip_prefix("gitdir: ")?.trim();
    Some(git_path.parent()?.join(relative))
}
