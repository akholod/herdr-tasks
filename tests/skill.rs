//! AC-7: the skill is printed verbatim, documents the CLI, and installs.

use assert_cmd::Command;

const SKILL: &str = include_str!("../skill/SKILL.md");

#[test]
fn skill_command_prints_file_byte_for_byte() {
    let out = Command::cargo_bin("herdr-tasks")
        .unwrap()
        .arg("skill")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, SKILL.as_bytes());
}

#[test]
fn skill_documents_cli_limit_and_details() {
    for term in [
        "create",
        "list",
        "show",
        "update",
        "move",
        "pin",
        "attach",
        "sync",
        "skill",
        "project",
        "description_limit",
        "details.md",
        "--json",
        "--append-details",
        "--details-file",
    ] {
        assert!(SKILL.contains(term), "SKILL.md lacks `{term}`");
    }
    assert!(SKILL.starts_with("---\nname: herdr-tasks\n"));
}

#[test]
fn install_script_copies_skill_and_binds_key_once() {
    let home = tempfile::tempdir().unwrap();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/install.sh");
    for _ in 0..2 {
        let out = std::process::Command::new("bash")
            .args([script, "--yes", "--no-link", "--no-build"])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let installed =
        std::fs::read_to_string(home.path().join(".claude/skills/herdr-tasks/SKILL.md")).unwrap();
    assert_eq!(installed, SKILL);
    let cfg = std::fs::read_to_string(home.path().join(".config/herdr/config.toml")).unwrap();
    assert_eq!(cfg.matches("herdr-tasks.open-dock").count(), 1);
    assert!(cfg.contains("key = \"prefix+t\""));
}
