//! The block of a user's `authorized_keys` that lets the roster's members in. It lists each member's
//! node key (a full shell as this user) and its input key (only `dojo-mesh inject`, as that member).
//! The block sits between two marker lines and is replaced or removed whole; the lines around it are
//! never touched, and the mesh never edits `sshd_config`.

use crate::device::Device;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

pub const BEGIN: &str = "# BEGIN dojo-mesh (managed by dojo-mesh; edit with dojo-mesh roster authorize)";
pub const END: &str = "# END dojo-mesh";

/// The lines for the members, without the markers. `inject` is the absolute path of `dojo-mesh` that
/// the input keys are forced to run. Members without keys (another method's) are left out, and so is
/// the node `this`: it does not let itself in.
pub fn lines(members: &[&Device], this: &str, inject: &str) -> Result<Vec<String>, String> {
    if inject.contains(['"', '\\', '\n', '\r']) || !Path::new(inject).is_absolute() {
        return Err(format!("{inject:?} cannot be put in a forced command"));
    }
    let mut out = Vec::new();
    for device in members {
        let Some(keys) = &device.keys else { continue };
        if device.name == this {
            continue;
        }
        out.push(format!("{} dojo-mesh:{}", keys.node_key, device.name));
        if let Some(input) = &keys.input_key {
            out.push(format!("restrict,command=\"{inject} inject --as {name}\" {input} dojo-mesh:{name}:input", name = device.name));
        }
    }
    Ok(out)
}

/// `existing` with the managed block set to `lines`: replaced where it is, appended when it is not,
/// and removed when there is nothing to put in it. A damaged block (a begin without an end) is an
/// error rather than a guess about what to cut.
pub fn replace_block(existing: &str, lines: &[String]) -> Result<String, String> {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut state = 0;
    for line in existing.lines() {
        match (state, line.trim_end()) {
            (0, BEGIN) => state = 1,
            (1, END) => state = 2,
            (1, _) => {}
            (0, _) => before.push(line),
            (_, _) => after.push(line),
        }
    }
    if state == 1 {
        return Err("authorized_keys has a dojo-mesh block with no end marker; fix it by hand before it is changed".into());
    }
    let mut out: Vec<String> = before.into_iter().map(String::from).collect();
    if !lines.is_empty() {
        out.push(BEGIN.into());
        out.extend(lines.iter().cloned());
        out.push(END.into());
    }
    out.extend(after.into_iter().map(String::from));
    Ok(if out.is_empty() { String::new() } else { out.join("\n") + "\n" })
}

/// Writes the block into `path` (`~/.ssh/authorized_keys`), keeping a copy of what was there. `true`
/// when the file changed.
pub fn apply(path: &Path, lines: &[String]) -> Result<bool, String> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let updated = replace_block(&existing, lines)?;
    if updated == existing {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    if !existing.is_empty() {
        let backup = path.with_extension("dojo-mesh.bak");
        std::fs::write(&backup, &existing).map_err(|error| format!("{}: {error}", backup.display()))?;
    }
    let temp = path.with_extension("dojo-mesh.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp).map_err(|error| format!("{}: {error}", temp.display()))?;
    std::io::Write::write_all(&mut file, updated.as_bytes()).map_err(|error| format!("{}: {error}", temp.display()))?;
    drop(file);
    std::fs::rename(&temp, path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Keys, device};

    fn member(name: &str, input: bool) -> Device {
        let mut d = device(name, &[], true);
        d.keys = Some(Keys { node_key: format!("ssh-ed25519 AAAA{name}"), input_key: input.then(|| format!("ssh-ed25519 AAAAinput{name}")), host_key: "ssh-ed25519 AAAAhost".into() });
        d
    }

    #[test]
    fn members_get_a_shell_and_their_input_key_runs_only_inject_as_them() {
        let (a, b, c) = (member("alpha", true), member("beta", true), member("gamma", false));
        let out = lines(&[&a, &b, &c], "alpha", "/home/pat/.local/bin/dojo-mesh").unwrap();
        assert_eq!(
            out,
            [
                "ssh-ed25519 AAAAbeta dojo-mesh:beta",
                "restrict,command=\"/home/pat/.local/bin/dojo-mesh inject --as beta\" ssh-ed25519 AAAAinputbeta dojo-mesh:beta:input",
                "ssh-ed25519 AAAAgamma dojo-mesh:gamma",
            ]
        );
    }

    #[test]
    fn a_path_that_could_break_out_of_the_forced_command_is_refused() {
        let a = member("beta", true);
        for bad in ["relative/dojo-mesh", "/x\" ; evil", "/x\\y", "/x\ny"] {
            assert!(lines(&[&a], "alpha", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_block_is_added_replaced_and_removed_without_touching_other_lines() {
        let mine = vec!["ssh-ed25519 AAAAbeta dojo-mesh:beta".to_string()];
        let added = replace_block("ssh-rsa AAAAmine me\n", &mine).unwrap();
        assert_eq!(added, format!("ssh-rsa AAAAmine me\n{BEGIN}\nssh-ed25519 AAAAbeta dojo-mesh:beta\n{END}\n"));
        let more = vec!["ssh-ed25519 AAAAgamma dojo-mesh:gamma".to_string()];
        let replaced = replace_block(&format!("{added}ssh-rsa AAAAlater later\n"), &more).unwrap();
        assert_eq!(replaced, format!("ssh-rsa AAAAmine me\n{BEGIN}\nssh-ed25519 AAAAgamma dojo-mesh:gamma\n{END}\nssh-rsa AAAAlater later\n"));
        assert_eq!(replace_block(&replaced, &[]).unwrap(), "ssh-rsa AAAAmine me\nssh-rsa AAAAlater later\n");
        assert_eq!(replace_block("", &[]).unwrap(), "");
    }

    #[test]
    fn a_block_with_no_end_is_not_guessed_at() {
        assert!(replace_block(&format!("ssh-rsa A me\n{BEGIN}\nssh-ed25519 AAAAx x\n"), &[]).is_err());
    }

    #[test]
    fn applying_keeps_a_backup_is_private_and_reports_no_change_when_there_is_none() {
        let dir = std::env::temp_dir().join(format!("dojo-authorize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(".ssh/authorized_keys");
        let mine = vec!["ssh-ed25519 AAAAbeta dojo-mesh:beta".to_string()];
        assert!(apply(&path, &mine).unwrap());
        assert!(!apply(&path, &mine).unwrap());
        std::fs::write(&path, format!("ssh-rsa A me\n{}", std::fs::read_to_string(&path).unwrap())).unwrap();
        assert!(apply(&path, &[]).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ssh-rsa A me\n");
        assert!(std::fs::read_to_string(path.with_extension("dojo-mesh.bak")).unwrap().contains("dojo-mesh:beta"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
