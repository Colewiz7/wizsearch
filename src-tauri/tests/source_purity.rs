//! Enforces the modularity invariant IN CODE: sources are pure. They may not
//! touch the shell, filesystem, database, credentials store, network clients,
//! or tauri itself. They get everything through SourceContext.

use std::fs;
use std::path::Path;

const FORBIDDEN: &[(&str, &str)] = &[
    ("std::fs", "filesystem access"),
    ("std::process", "shell/process access"),
    ("std::net", "raw network access"),
    ("tokio::fs", "filesystem access"),
    ("tokio::process", "shell/process access"),
    ("tokio::net", "raw network access"),
    ("reqwest", "http client (use ctx.http())"),
    ("rusqlite", "database access"),
    ("keyring", "credential store (use ctx.credential())"),
    ("tauri", "app/host access"),
    ("Command::new", "shell/process access"),
    ("std::env", "environment access"),
];

#[test]
fn sources_are_pure() {
    let sources_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sources");
    let mut checked = 0;
    let mut violations = Vec::new();

    for entry in fs::read_dir(&sources_dir).expect("src/sources must exist") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let content = fs::read_to_string(&path).expect("readable source file");
        checked += 1;

        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for (token, why) in FORBIDDEN {
                if trimmed.contains(token) {
                    violations.push(format!("{name}:{}: '{token}' ({why})", line_no + 1));
                }
            }
        }
    }

    assert!(
        checked >= 4,
        "expected mod.rs + at least 3 sources, found {checked}"
    );
    assert!(
        violations.is_empty(),
        "sources must stay pure (data in, data + fetch plans out):\n{}",
        violations.join("\n")
    );
}
