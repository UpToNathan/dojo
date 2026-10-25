use chrono::Utc;
use dojo_mesh::config::{MeshConfig, default_path};
use dojo_mesh::doctor::{Severity, diagnose};
use dojo_mesh::{report, tailscale};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: dojo-mesh [--config PATH] status [--all] [--json]\n       dojo-mesh [--config PATH] doctor [--json]\n";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("dojo-mesh: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(mut args: Vec<String>) -> Result<ExitCode, String> {
    let mut config_path: Option<PathBuf> = None;
    if args.first().map(String::as_str) == Some("--config") {
        if args.len() < 2 {
            return Err(format!("--config needs a path\n{USAGE}"));
        }
        config_path = Some(args[1].clone().into());
        args.drain(..2);
    }
    let Some(command) = args.first().cloned() else { return Err(USAGE.trim_end().into()) };
    let flags = &args[1..];
    let json = flags.iter().any(|flag| flag == "--json");
    let all = flags.iter().any(|flag| flag == "--all");
    let allowed: &[&str] = if command == "status" { &["--json", "--all"] } else { &["--json"] };
    if let Some(unknown) = flags.iter().find(|flag| !allowed.contains(&flag.as_str())) {
        return Err(format!("unknown option {unknown}\n{USAGE}"));
    }

    let config = match config_path.or_else(|| default_path(std::env::var_os("XDG_CONFIG_HOME").map(Into::into), std::env::var_os("HOME").map(Into::into))) {
        Some(path) => MeshConfig::load(&path).map_err(|error| error.to_string())?,
        None => MeshConfig::default(),
    };
    let fallback = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|name| name.trim().to_string()).unwrap_or_default();
    let roster = tailscale::roster(tailscale::read_status(), &config, if fallback.is_empty() { "this-node" } else { &fallback });

    match command.as_str() {
        "status" => {
            if json {
                println!("{}", serde_json::to_string_pretty(&roster).map_err(|error| error.to_string())?);
            } else {
                print!("{}", report::status(&roster, all));
            }
            Ok(ExitCode::SUCCESS)
        }
        "doctor" => {
            let findings = diagnose(&roster, &config, Utc::now());
            if json {
                println!("{}", serde_json::to_string_pretty(&findings).map_err(|error| error.to_string())?);
            } else {
                print!("{}", report::doctor(&findings));
            }
            Ok(if findings.iter().any(|finding| finding.severity == Severity::Problem) { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}
