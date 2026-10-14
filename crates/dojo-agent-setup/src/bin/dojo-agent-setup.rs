use dojo_agent_setup::{AgentSetup, HostEnv, McpServer, Outcome, Parts};
use std::path::PathBuf;

#[derive(Default)]
struct Options {
    apply: bool,
    remove: bool,
    force: bool,
    agents: String,
    root: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        agents: "all".into(),
        ..Options::default()
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => options.apply = true,
            "--remove" => options.remove = true,
            "--force" => options.force = true,
            "--plan" => {},
            "--agents" => options.agents = args.next().ok_or("--agents needs a value")?,
            "--root" | "--home" => {
                if options.root.is_some() { return Err("choose only one of --root or --home".into()); }
                options.root = Some(args.next().ok_or("--root needs a path")?.into());
            }
            "--help" | "-h" => return Err("usage: dojo-agent-setup [--agents all|id,id] [--root PATH] [--apply] [--remove] [--force]".into()),
            _ => return Err(format!("unknown argument {arg:?}")),
        }
    }
    if options.remove && options.force {
        return Err("--force applies only to setup".into());
    }
    Ok(options)
}

fn execute(
    args: impl IntoIterator<Item = String>,
    current: HostEnv,
) -> Result<(bool, Vec<Outcome>), String> {
    let options = parse_args(args)?;
    let env = options.root.map(HostEnv::isolated).unwrap_or(current);
    let setup = AgentSetup::new(env.clone());
    let agents = setup.select(&options.agents).map_err(|e| e.to_string())?;
    if options.remove {
        if options.apply {
            let outcomes = setup
                .remove(&agents, Parts::ALL)
                .map_err(|e| e.to_string())?;
            return Ok((true, outcomes));
        }
        return Ok((
            false,
            vec![Outcome {
                agents,
                target: dojo_agent_setup::Target::Mcp,
                item: "dojo setup state".into(),
                path: setup.state_path().to_path_buf(),
                change: dojo_agent_setup::Change::Updated,
                detail: "would remove only unchanged Dojo-owned entries and skills".into(),
                backup: None,
            }],
        ));
    }
    let server = McpServer::default_for(&env);
    if !options.apply {
        let outcomes = setup
            .plan(&agents, &server, options.force)
            .map_err(|e| e.to_string())?;
        return Ok((false, outcomes));
    }
    let mut outcomes = setup
        .install_skills(&agents, &[], options.force)
        .map_err(|e| e.to_string())?;
    outcomes.extend(
        setup
            .configure_mcp(&agents, &server)
            .map_err(|e| e.to_string())?,
    );
    Ok((true, outcomes))
}

fn main() {
    let args = std::env::args().skip(1);
    match execute(args, HostEnv::current()) {
        Ok((true, outcomes)) => {
            for outcome in outcomes {
                println!(
                    "{:?} {:?}: {} ({})",
                    outcome.change,
                    outcome.target,
                    outcome.path.display(),
                    outcome.detail
                );
            }
        }
        Ok((false, outcomes)) if outcomes.is_empty() => {
            println!(
                "Plan only. No tracked agents were selected. Pass --agents ID or specify --apply after reviewing the plan."
            );
        }
        Ok((false, outcomes)) => {
            println!("Plan only. No files were changed. Pass --apply after reviewing the plan.");
            for outcome in outcomes {
                println!(
                    "{:?}: {} ({})",
                    outcome.change,
                    outcome.path.display(),
                    outcome.detail
                );
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_plan_reads_sandbox_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".claude.json"), "{\"mcpServers\":{}}\n").unwrap();
        let (applied, outcomes) = execute(
            ["--root", root.to_str().unwrap(), "--agents", "claude-code"].map(str::to_owned),
            HostEnv::isolated("/unused"),
        )
        .unwrap();
        assert!(!applied);
        assert!(!outcomes.is_empty());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert!(!root.join(".dojo/agent-setup.json").exists());
        assert_eq!(
            std::fs::read_to_string(root.join(".claude.json")).unwrap(),
            "{\"mcpServers\":{}}\n"
        );
    }

    #[test]
    fn explicit_apply_writes_only_inside_requested_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".claude.json"), "{\"mcpServers\":{}}\n").unwrap();
        let (applied, outcomes) = execute(
            [
                "--root",
                root.to_str().unwrap(),
                "--agents",
                "claude-code",
                "--apply",
            ]
            .map(str::to_owned),
            HostEnv::isolated("/unused"),
        )
        .unwrap();
        assert!(applied);
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.path.starts_with(&root))
        );
        assert!(root.join(".dojo/agent-setup.json").is_file());
        assert!(
            std::fs::read_to_string(root.join(".claude.json"))
                .unwrap()
                .contains("dojo-desktop")
        );
        assert!(std::fs::read_dir(&root).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains(".dojo-backup-")
        }));
    }
}
