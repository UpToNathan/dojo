//! `dojo-mesh keys`: the keys this node signs with (in order) and trusts, and the changes to them:
//! move, stop signing with, stop trusting, and add (a key in 1Password, a security key, a key file).
//! The mesh panel runs these, so a key is never managed by editing a file.

use crate::cli::{Env, FAILED, Failure, USAGE_ERROR, fail, has, split};
use crate::enroll_cli::{self, Ctx};
use crate::keys::{self, Kind};
use std::path::{Path, PathBuf};

fn config_path(env: &Env) -> Result<&Path, Failure> {
    env.config_path.as_deref().ok_or_else(|| fail(USAGE_ERROR, "there is nowhere to keep the mesh configuration (HOME is not set)"))
}

fn set_signing(env: &Env, list: &[PathBuf]) -> Result<(), Failure> {
    keys::write_signing_keys(config_path(env)?, list).map_err(|message| fail(USAGE_ERROR, message))
}

fn public_of(path: &Path) -> Result<String, Failure> {
    let file = if path.extension().and_then(|ext| ext.to_str()) == Some("pub") { path.to_path_buf() } else { PathBuf::from(format!("{}.pub", path.display())) };
    enroll_cli::read_key(&file).map_err(|message| fail(USAGE_ERROR, message))
}

/// Adds `path` as a key this node signs with (last, so the existing primary stays primary) and trusts it.
fn adopt(env: &Env, ctx: &Ctx, path: PathBuf) -> Result<(), Failure> {
    let mut list = ctx.config.signing_keys.clone();
    if list.contains(&path) {
        return Err(fail(FAILED, "that key is already one this node signs with"));
    }
    enroll_cli::trust_key(ctx, &public_of(&path)?)?;
    list.push(path);
    set_signing(env, &list)
}

pub fn keys(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = enroll_cli::context(env)?;
    let (words, values) = split(rest, &["--json"], &["--vault"])?;
    let words_list: Vec<&str> = words.positional.iter().map(String::as_str).collect();
    match words_list.as_slice() {
        [] | ["list"] => {
            let rows = keys::list(&ctx.config, &ctx.paths.signers);
            if has(&words, "--json") {
                println!("{}", serde_json::to_string(&rows).map_err(|error| fail(FAILED, error.to_string()))?);
                return Ok(0);
            }
            if rows.is_empty() {
                println!("This node has no signing keys and trusts none. Add one: dojo-mesh keys add 1password");
            }
            for row in rows.iter().filter(|row| row.order.is_some()) {
                let place = row.order.unwrap_or(0);
                println!("{place}. {}{}{}", row.kind.label(), if place == 1 { "  (primary)" } else { "" }, if row.trusted { "" } else { "  (not trusted here)" });
            }
            for row in rows.iter().filter(|row| row.order.is_none()) {
                println!("   {}  (trusted, does not sign here)", row.kind.label());
            }
            Ok(0)
        }
        ["move", path, direction @ ("up" | "down")] => {
            let moved = keys::moved(&ctx.config.signing_keys, Path::new(path), *direction == "up").ok_or_else(|| fail(FAILED, format!("{path} cannot move {direction}")))?;
            set_signing(env, &moved)?;
            println!("moved {direction}");
            Ok(0)
        }
        ["unsign", path] => {
            let list: Vec<PathBuf> = ctx.config.signing_keys.iter().filter(|key| key.as_path() != Path::new(path)).cloned().collect();
            if list.len() == ctx.config.signing_keys.len() {
                return Err(fail(FAILED, format!("{path} is not a key this node signs with")));
            }
            set_signing(env, &list)?;
            println!("no longer signing with it");
            Ok(0)
        }
        ["distrust", fingerprint] => {
            let rows = keys::list(&ctx.config, &ctx.paths.signers);
            let row = rows.iter().find(|row| row.fingerprint == *fingerprint).ok_or_else(|| fail(FAILED, "no such key"))?;
            if let Some(path) = &row.path {
                let list: Vec<PathBuf> = ctx.config.signing_keys.iter().filter(|key| *key != path).cloned().collect();
                set_signing(env, &list)?;
            }
            enroll_cli::trust(env, &["remove".to_string(), fingerprint.to_string()])?;
            Ok(0)
        }
        ["add", "1password"] => {
            let dir = ctx.paths.node_key.parent().map(Path::to_path_buf).ok_or_else(|| fail(FAILED, "the node key has no directory"))?;
            let name = enroll_cli::node_name(env, &ctx.config);
            let key = enroll_cli::onepassword_key(&format!("dojo-mesh signing key ({name})"), enroll_cli::value(&values, "--vault").as_deref())?;
            let mut path = dir.join("signing_1password.pub");
            let mut number = 2;
            while path.exists() {
                path = dir.join(format!("signing_1password_{number}.pub"));
                number += 1;
            }
            enroll_cli::write_private(&path, &format!("{key} dojo-mesh:{name}:signing\n"), 0o644)?;
            adopt(env, &ctx, path)?;
            println!("added a {}", Kind::OnePassword.label());
            Ok(0)
        }
        ["add", "security-key"] => {
            let dir = ctx.paths.node_key.parent().map(Path::to_path_buf).ok_or_else(|| fail(FAILED, "the node key has no directory"))?;
            let name = enroll_cli::node_name(env, &ctx.config);
            let path = crate::onboard::yubikey_key(&dir, &name)?;
            adopt(env, &ctx, path)?;
            println!("added a {}", Kind::SecurityKey.label());
            Ok(0)
        }
        ["add", "file", path] => {
            let path = PathBuf::from(path);
            if !path.is_absolute() || !path.exists() {
                return Err(fail(USAGE_ERROR, "give the absolute path of a key file that exists"));
            }
            adopt(env, &ctx, path)?;
            println!("added a {}", Kind::File.label());
            Ok(0)
        }
        _ => Err(fail(USAGE_ERROR, "usage: dojo-mesh keys [list [--json] | move PATH up|down | unsign PATH | distrust FINGERPRINT | add 1password [--vault V] | add security-key | add file PATH]")),
    }
}
