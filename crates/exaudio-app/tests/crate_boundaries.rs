//! The editor must build and run without the optional extensions. `crates/` holds the editor and
//! its platform; `extensions/` holds everything optional (Babel, AI, formats, checks, modes).
//! Only `exaudio-app`, the composition root, may name extension crates, and the extensions of
//! the editor-only build may not pull in other extensions.

use std::{fs, path::Path};

/// Extensions in the editor-only build (`cargo editor`).
const EDITOR_ONLY_EXTENSIONS: [&str; 4] = ["palette", "local", "formats", "lint-basic"];

fn manifests(dir: &Path) -> Vec<(String, String)> {
    let mut manifests: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(|entry| {
            let entry = entry.expect("directory entry");
            let manifest = fs::read_to_string(entry.path().join("Cargo.toml")).ok()?;
            Some((entry.file_name().to_string_lossy().into_owned(), manifest))
        })
        .collect();
    manifests.sort();
    manifests
}

fn package_name(manifest: &str) -> &str {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .find_map(|line| line.trim().strip_prefix("name = \"")?.strip_suffix('"'))
        .expect("package name")
}

/// Whether the manifest lists `package` as a dependency key (`name = …`, `name.workspace = …`).
fn depends_on(manifest: &str, package: &str) -> bool {
    manifest.lines().any(|line| {
        line.trim()
            .strip_prefix(package)
            .is_some_and(|rest| rest.starts_with([' ', '.', '=']))
    })
}

#[test]
fn only_the_app_crate_depends_on_extensions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let extensions = manifests(&root.join("extensions"));
    let mut violations = Vec::new();
    let mut check = |kind: &str, dir: &str, manifest: &str| {
        for (extension_dir, extension) in &extensions {
            let package = package_name(extension);
            if extension_dir != dir && depends_on(manifest, package) {
                violations.push(format!("{kind}/{dir} depends on {package}"));
            }
        }
    };
    for (dir, manifest) in manifests(&root.join("crates")) {
        if dir != "exaudio-app" {
            check("crates", &dir, &manifest);
        }
    }
    for (dir, manifest) in &extensions {
        if EDITOR_ONLY_EXTENSIONS.contains(&dir.as_str()) {
            check("extensions", dir, manifest);
        }
    }
    assert!(
        violations.is_empty(),
        "the editor-only build must not need optional extensions:\n{}",
        violations.join("\n")
    );
}
