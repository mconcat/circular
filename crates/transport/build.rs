use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

fn watch(path: impl AsRef<Path>) {
    println!("cargo:rerun-if-changed={}", path.as_ref().display());
}

fn git_identity(root: &Path) -> Option<(String, bool, String)> {
    if !root.join(".git").exists() {
        return None;
    }
    let sha = git(root, &["rev-parse", "HEAD"])?;
    let dirty = !git(root, &["status", "--porcelain"])?.trim().is_empty();
    let date = git(root, &["show", "-s", "--format=%cs", "HEAD"])?;
    for name in ["HEAD", "index", "packed-refs"] {
        let path = git(root, &["rev-parse", "--git-path", name])?;
        watch(root.join(path.trim()));
    }
    if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        let path = git(root, &["rev-parse", "--git-path", reference.trim()])?;
        watch(root.join(path.trim()));
    }
    Some((
        sha.trim().chars().take(12).collect(),
        dirty,
        date.trim().into(),
    ))
}

fn archive_identity(root: &Path) -> (String, bool, String) {
    let mut identity = ("unknown".to_owned(), false, "unknown".to_owned());
    if let Ok(info) = fs::read_to_string(root.join("BUILD_INFO")) {
        for line in info.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "git_sha" if !value.is_empty() => identity.0 = value.chars().take(12).collect(),
                "git_dirty" => identity.1 = value == "true",
                "commit_date" if !value.is_empty() => identity.2 = value.into(),
                _ => {}
            }
        }
    }
    identity
}

fn main() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    watch(manifest.join("build.rs"));
    watch(root.join(".git"));
    watch(root.join("BUILD_INFO"));
    let (sha, dirty, date) = git_identity(root).unwrap_or_else(|| {
        println!("cargo:warning=Git build identity unavailable; using BUILD_INFO or unknown");
        archive_identity(root)
    });
    println!("cargo:rustc-env=BUILD_GIT_SHA={sha}");
    println!("cargo:rustc-env=BUILD_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=BUILD_COMMIT_DATE={date}");
    println!(
        "cargo:rustc-env=BUILD_TARGET={}",
        env::var("TARGET").expect("Cargo target")
    );
}
