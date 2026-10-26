//! `dojo-mesh onboard`: everything a machine needs to take part in a roster mesh, in order and safe to
//! repeat. It writes the configuration, gives the node its keys, makes or finds the keys it signs
//! with, trusts them, and either enrols the node (a node that signs enrols itself) or writes the
//! request for an operator to sign. Each step is the command that does it alone, so what happens
//! here is what the person could do by hand, with the same checks.

use crate::cli::{Env, FAILED, Failure, NEEDS_HUMAN, USAGE_ERROR, fail, split};
use crate::config::set_top_level;
use crate::enroll;
use crate::enroll_cli::{self, Ctx};
use crate::signed;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

const YUBIKEY_FILE: &str = "signing_yubikey";
const ONEPASSWORD_FILE: &str = "signing_1password.pub";
/// What a hardware credential made here is named on the key, so it can be found again on a new machine.
const YUBIKEY_APPLICATION: &str = "ssh:dojo-mesh";

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn write_config(path: &Path, text: &str) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| fail(FAILED, format!("{}: {error}", parent.display())))?;
    }
    std::fs::write(path, text).map_err(|error| fail(FAILED, format!("{}: {error}", path.display())))
}

fn setting(error: String) -> Failure {
    fail(USAGE_ERROR, error)
}

/// The public half of a signing key given as a private key file or a public one.
fn public_of(key: &Path) -> Result<String, Failure> {
    let public = if key.extension().and_then(|ext| ext.to_str()) == Some("pub") { key.to_path_buf() } else { PathBuf::from(format!("{}.pub", key.display())) };
    enroll_cli::read_key(&public).map_err(|message| fail(FAILED, message))
}

/// Looks for a hardware credential this mesh made earlier on the key that is plugged in, so a second
/// machine uses the same signing key instead of making another.
fn recover_yubikey(dir: &Path, target: &Path) -> bool {
    let scratch = dir.join(".recover");
    let _ = std::fs::remove_dir_all(&scratch);
    if std::fs::create_dir_all(&scratch).is_err() {
        return false;
    }
    eprintln!("Looking on the YubiKey for a signing key made earlier (touch it, and enter its PIN if asked)…");
    let ran = Command::new("ssh-keygen").arg("-K").current_dir(&scratch).status().is_ok_and(|status| status.success());
    let mut found = false;
    if ran {
        if let Ok(entries) = std::fs::read_dir(&scratch) {
            for entry in entries.flatten() {
                let file = entry.path();
                let Some(file_name) = file.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
                // Resident keys come back as id_ed25519_sk_rk_<application>[_<user>], with a .pub beside each.
                if file_name.starts_with("id_ed25519_sk_rk_dojo-mesh") && !file_name.ends_with(".pub") && std::fs::rename(&file, target).is_ok() {
                    found = std::fs::rename(scratch.join(format!("{file_name}.pub")), format!("{}.pub", target.display())).is_ok();
                    break;
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    found
}

/// The signing key on the YubiKey: found from an earlier run, taken back from the key itself, or made.
fn yubikey_key(dir: &Path, name: &str) -> Result<PathBuf, Failure> {
    let path = dir.join(YUBIKEY_FILE);
    if path.exists() && PathBuf::from(format!("{}.pub", path.display())).exists() {
        return Ok(path);
    }
    // A security key usually has a PIN, and every wrong answer counts against the key's own limit, so
    // `ssh-keygen` is not started where nobody can type it: it would try an empty PIN and be locked out.
    if !(std::io::stdin().is_terminal() || std::env::var_os("SSH_ASKPASS").is_some_and(|program| !program.is_empty())) {
        return Err(fail(NEEDS_HUMAN, "the YubiKey may ask for its PIN, and there is no terminal to ask on: run this command yourself in a terminal"));
    }
    std::fs::create_dir_all(dir).map_err(|error| fail(FAILED, format!("{}: {error}", dir.display())))?;
    if recover_yubikey(dir, &path) {
        println!("Took this mesh's signing key back from the YubiKey.");
        return Ok(path);
    }
    eprintln!("Making a signing key on the YubiKey: touch it when it blinks (and enter its PIN if asked).");
    let status = Command::new("ssh-keygen")
        .args(["-t", "ed25519-sk", "-O", "resident", "-O"])
        .arg(format!("application={YUBIKEY_APPLICATION}"))
        .arg("-C")
        .arg(format!("dojo-mesh signing key ({name})"))
        .args(["-N", "", "-f"])
        .arg(&path)
        .status()
        .map_err(|error| fail(FAILED, format!("could not run ssh-keygen: {error}")))?;
    if !status.success() || !path.exists() {
        return Err(fail(NEEDS_HUMAN, "the YubiKey did not make the key: plug it in, touch it when it blinks, and run this again (ssh-keygen needs a terminal if the key has a PIN)"));
    }
    Ok(path)
}

/// The signing key held by 1Password's agent: found from an earlier run, or made there.
fn onepassword_signing_key(dir: &Path, name: &str, vault: Option<&str>) -> Result<PathBuf, Failure> {
    let path = dir.join(ONEPASSWORD_FILE);
    if path.exists() {
        return Ok(path);
    }
    let key = enroll_cli::onepassword_key(&format!("dojo-mesh signing key ({name})"), vault)?;
    enroll_cli::write_private(&path, &format!("{key} dojo-mesh:{name}:signing\n"), 0o644)?;
    Ok(path)
}

fn signing_key(spec: &str, dir: &Path, name: &str, vault: Option<&str>) -> Result<PathBuf, Failure> {
    match spec {
        "yubikey" => yubikey_key(dir, name),
        "1password" => onepassword_signing_key(dir, name, vault),
        path if Path::new(path).is_absolute() && Path::new(path).exists() => Ok(PathBuf::from(path)),
        other => Err(fail(USAGE_ERROR, format!("--sign takes yubikey, 1password or the absolute path of a key file, not {other:?}"))),
    }
}

fn describe(path: &Path) -> String {
    match public_of(path).ok().and_then(|key| enroll::fingerprint(&key).ok().map(|fp| (fp, enroll::held_as(&key)))) {
        Some((fp, held)) => format!("{}  {fp}  ({held})", path.display()),
        None => path.display().to_string(),
    }
}

pub fn onboard(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let (words, values) = split(rest, &["--invite"], &["--node-name", "--route", "--sign", "--vault", "--host-key-file", "--out", "--wait"])?;
    if !words.positional.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh onboard [--node-name NAME] [--route HOST[:PORT]]... [--sign yubikey|1password|KEY]... [--vault NAME]"));
    }
    let config_path = env.config_path.clone().ok_or_else(|| fail(USAGE_ERROR, "there is nowhere to keep the mesh configuration (HOME is not set)"))?;
    let name = enroll_cli::value(&values, "--node-name").unwrap_or_else(|| env.hostname.trim().to_ascii_lowercase());
    signed::valid_name(&name).map_err(|reason| fail(USAGE_ERROR, format!("{reason}; give the node a name with --node-name")))?;
    let vault = enroll_cli::value(&values, "--vault");
    let specs: Vec<&str> = values.iter().filter(|(flag, _)| flag == "--sign").map(|(_, spec)| spec.as_str()).collect();
    let routes: Vec<String> = values.iter().filter(|(flag, _)| flag == "--route").map(|(_, route)| route.clone()).collect();

    // 1. The configuration says this node uses the roster, under this name.
    println!("== configuration ({})", config_path.display());
    let mut text = std::fs::read_to_string(&config_path).unwrap_or_default();
    text = set_top_level(&text, "method", "\"roster\"").map_err(setting)?;
    text = set_top_level(&text, "node-name", &quoted(&name)).map_err(setting)?;
    write_config(&config_path, &text)?;
    let mut env = env.reloaded();
    let ctx = enroll_cli::context(&env)?;
    let dir = ctx.paths.node_key.parent().map(Path::to_path_buf).ok_or_else(|| fail(FAILED, "the node key has no directory"))?;

    // 2. This node's own keys, in 1Password.
    println!("== this node's keys");
    if ctx.paths.node_key.exists() {
        println!("{} already has its keys ({}).", name, ctx.paths.node_key.display());
    } else {
        let mut init = vec!["--custody".to_string(), "1password".to_string()];
        if let Some(vault) = &vault {
            init.extend(["--vault".to_string(), vault.clone()]);
        }
        enroll_cli::init(&env, &init)?;
    }

    // 3. The keys it signs with, tried in the order given; the first is the primary.
    let mut signing = Vec::new();
    if !specs.is_empty() {
        println!("== signing keys");
        for spec in &specs {
            let key = signing_key(spec, &dir, &name, vault.as_deref())?;
            println!("{spec}: {}", describe(&key));
            signing.push(key);
        }
        let list = format!("[{}]", signing.iter().map(|key| quoted(&key.display().to_string())).collect::<Vec<_>>().join(", "));
        text = set_top_level(&text, "signing-keys", &list).map_err(setting)?;
        write_config(&config_path, &text)?;
        env = env.reloaded();
    }

    // 4. This node believes the keys it signs with: it made them, or took them from its own device.
    let ctx = enroll_cli::context(&env)?;
    for key in &signing {
        let public = public_of(key)?;
        let fp = enroll::fingerprint(&public).map_err(|message| fail(FAILED, message))?;
        let pub_path = if key.extension().and_then(|ext| ext.to_str()) == Some("pub") { key.clone() } else { PathBuf::from(format!("{}.pub", key.display())) };
        enroll_cli::trust(&env, &["add".into(), pub_path.display().to_string(), "--fingerprint".into(), fp])?;
    }

    // 5. A node that signs enrols itself; any other writes a request for one that does.
    println!("== enrolment");
    let mut join = Vec::new();
    for route in &routes {
        join.extend(["--route".to_string(), route.clone()]);
    }
    if let Some(file) = enroll_cli::value(&values, "--host-key-file") {
        join.extend(["--host-key-file".to_string(), file]);
    }
    if ctx.config.signing_keys.is_empty() && crate::cli::has(&words, "--invite") {
        // Through an invite: nothing to carry. The machine already in the mesh presses Add machine (or runs
        // `dojo-mesh invite`), and this one finds it, asks, and installs the answer.
        join.push("--invite".to_string());
        if let Some(wait) = enroll_cli::value(&values, "--wait") {
            join.extend(["--wait".to_string(), wait]);
        }
        println!("Looking for an invite from a machine in the mesh (Add machine in its mesh panel, or dojo-mesh invite)…");
        return enroll_cli::join(&env, &join);
    }
    if ctx.config.signing_keys.is_empty() {
        let out = enroll_cli::value(&values, "--out").map(PathBuf::from).unwrap_or_else(|| dir.join(format!("{name}.join.json")));
        join.extend(["--out".to_string(), out.display().to_string()]);
        enroll_cli::join(&env, &join)?;
        println!("\nThis node signs nothing, so an operator has to sign it in:");
        println!("  1. give {} to a node that signs, and compare the fingerprints above", out.display());
        println!("  2. there: dojo-mesh approve {} --fingerprint SHA256:… ", out.file_name().and_then(|n| n.to_str()).unwrap_or("REQUEST"));
        println!("  3. here: dojo-mesh trust add THE_SIGNING_KEY.pub --fingerprint SHA256:…  (once per signing key), then copy or `dojo-mesh roster sync` the roster");
        println!("  (or, with no files: `dojo-mesh invite` there, and `dojo-mesh join --invite` here)");
        return Ok(0);
    }
    enroll_self(&env, &ctx, &name, join)?;
    println!("\n{name} is set up. `dojo-mesh doctor` shows how it looks from here.");
    Ok(0)
}

/// Signs this node into its own roster, unless it is there already.
fn enroll_self(env: &Env, ctx: &Ctx, name: &str, mut join: Vec<String>) -> Result<(), Failure> {
    if enroll_cli::device_file(ctx, name).exists() {
        println!("{name} is already in the roster.");
        return Ok(());
    }
    let request = ctx.paths.node_key.parent().unwrap_or(Path::new(".")).join(format!("{name}.join.json"));
    join.extend(["--out".to_string(), request.display().to_string()]);
    enroll_cli::join(env, &join)?;
    let node = enroll_cli::read_key(&ctx.paths.node_key).map_err(|message| fail(FAILED, message))?;
    let fp = enroll::fingerprint(&node).map_err(|message| fail(FAILED, message))?;
    println!("Signing {name} into the roster (touch the YubiKey if it blinks)…");
    let result = enroll_cli::approve(env, &[request.display().to_string(), "--fingerprint".into(), fp]);
    let _ = std::fs::remove_file(&request);
    result.map(|_| ())
}

