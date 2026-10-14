//! Integration tests use only a temporary home. They never inspect or change
//! the real user's agent configuration.
use dojo_agent_setup::{
    AGENTS, AgentSetup, Change, Format, HostEnv, McpServer, Parts, Target, edit_get,
};
use std::path::{Path, PathBuf};

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    env: HostEnv,
}
impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let env = HostEnv::isolated(&home);
        Self {
            _dir: dir,
            home,
            env,
        }
    }
    fn write(&self, path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}
fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

#[test]
fn every_registered_mcp_format_installs_idempotently_and_removes_only_dojo_entry() {
    let server = McpServer::new("/opt/dojo/bin/dojo-agent");
    let with_mcp: Vec<_> = AGENTS.iter().filter(|agent| agent.mcp.is_some()).collect();
    assert!(
        with_mcp.len() >= 14,
        "registry lost supported agent formats"
    );
    for agent in with_mcp {
        let sandbox = Sandbox::new();
        let setup = AgentSetup::new(sandbox.env.clone());
        let spec = agent.mcp.unwrap();
        let path = spec
            .alt_files
            .iter()
            .map(|loc| loc.resolve(&sandbox.env))
            .find(|path| path.exists())
            .unwrap_or_else(|| spec.file.resolve(&sandbox.env));
        let original = match spec.format {
            Format::Json => "{\n  // keep this comment\n  \"unrelated\": true\n}\n",
            Format::Toml => "# keep this comment\nmodel = \"fixture\"\n",
            Format::Yaml => "# keep this comment\nmodel: fixture\n",
        };
        sandbox.write(&path, original);
        let outcomes = setup.configure_mcp(&ids(&[agent.id]), &server).unwrap();
        assert_eq!(outcomes.len(), 1, "{}: {outcomes:#?}", agent.id);
        assert_eq!(outcomes[0].change, Change::Created, "{}", agent.id);
        let edited = std::fs::read_to_string(&path).unwrap();
        assert!(edited.contains("dojo-agent"), "{}:\n{edited}", agent.id);
        assert!(
            edited.contains("unrelated") || edited.contains("model"),
            "{}:\n{edited}",
            agent.id
        );
        assert!(
            edited.contains("keep this comment"),
            "{}:\n{edited}",
            agent.id
        );
        let expected =
            edit_get(spec.format, &path, &edited, spec.key_path, "dojo-desktop").unwrap();
        assert!(expected.is_some(), "{}:\n{edited}", agent.id);
        assert!(
            outcomes[0]
                .backup
                .as_ref()
                .is_some_and(|backup| backup.is_file())
        );
        assert!(
            setup
                .detect()
                .iter()
                .find(|status| status.id == agent.id)
                .unwrap()
                .dojo_configured
        );

        let second = setup.configure_mcp(&ids(&[agent.id]), &server).unwrap();
        assert_eq!(second[0].change, Change::Unchanged, "{}", agent.id);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);

        let removed = setup
            .remove(
                &ids(&[agent.id]),
                Parts {
                    skills: false,
                    mcp: true,
                },
            )
            .unwrap();
        assert_eq!(
            removed[0].change,
            Change::Removed,
            "{}: {removed:#?}",
            agent.id
        );
        let restored = std::fs::read_to_string(&path).unwrap();
        assert!(
            restored.contains("keep this comment"),
            "{}:\n{restored}",
            agent.id
        );
        assert!(
            edit_get(spec.format, &path, &restored, spec.key_path, "dojo-desktop")
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn skill_install_skips_foreign_content_and_force_keeps_a_backup() {
    let sandbox = Sandbox::new();
    let setup = AgentSetup::new(sandbox.env.clone());
    let agent_ids = ids(&["kiro"]);
    let dest = sandbox.home.join(".kiro/skills/dojo-desktop");
    sandbox.write(&dest.join("SKILL.md"), "local customization\n");

    let skipped = setup.install_skills(&agent_ids, &[], false).unwrap();
    assert_eq!(skipped[0].change, Change::Skipped);
    assert_eq!(
        std::fs::read_to_string(dest.join("SKILL.md")).unwrap(),
        "local customization\n"
    );

    let installed = setup.install_skills(&agent_ids, &[], true).unwrap();
    assert_eq!(installed[0].change, Change::Updated);
    let backup = installed[0].backup.as_ref().unwrap();
    assert_eq!(
        std::fs::read_to_string(backup.join("SKILL.md")).unwrap(),
        "local customization\n"
    );
    assert!(
        std::fs::read_to_string(dest.join("SKILL.md"))
            .unwrap()
            .contains("name: dojo-desktop")
    );

    let removed = setup
        .remove(
            &agent_ids,
            Parts {
                skills: true,
                mcp: false,
            },
        )
        .unwrap();
    assert_eq!(removed[0].change, Change::Removed);
    assert!(backup.is_dir());
}

#[test]
fn isolated_detection_and_plan_are_read_only() {
    let sandbox = Sandbox::new();
    let setup = AgentSetup::new(sandbox.env.clone());
    let config = sandbox.home.join(".claude.json");
    sandbox.write(&config, "{\"mcpServers\":{}}\n");
    let before = std::fs::read(&config).unwrap();
    let plan = setup
        .plan(
            &ids(&["claude-code"]),
            &McpServer::default_for(&sandbox.env),
            false,
        )
        .unwrap();
    assert!(
        plan.iter()
            .any(|outcome| outcome.target == Target::Mcp && outcome.change == Change::Created)
    );
    assert_eq!(std::fs::read(&config).unwrap(), before);
    assert!(!setup.state_path().exists());

    let bin_dir = sandbox.home.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let agent_bin = bin_dir.join("claude");
    sandbox.write(&agent_bin, "#!/bin/sh\nexit 0\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&agent_bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut env = sandbox.env.clone();
    env.path.push(bin_dir);
    let detected = AgentSetup::new(env)
        .detect()
        .into_iter()
        .find(|status| status.id == "claude-code")
        .unwrap();
    assert!(detected.installed);
    assert!(
        detected
            .evidence
            .iter()
            .any(|entry| entry.starts_with("bin:"))
    );
}

#[test]
fn malformed_config_is_reported_without_a_write_or_backup() {
    let sandbox = Sandbox::new();
    let setup = AgentSetup::new(sandbox.env.clone());
    let path = sandbox.home.join(".claude.json");
    sandbox.write(&path, "{\"mcpServers\": {");
    let before = std::fs::read(&path).unwrap();
    let result = setup
        .configure_mcp(
            &ids(&["claude-code"]),
            &McpServer::default_for(&sandbox.env),
        )
        .unwrap();
    assert_eq!(result[0].change, Change::Failed);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(&sandbox.home).unwrap().count(), 1);
    assert!(!setup.state_path().exists());
}
