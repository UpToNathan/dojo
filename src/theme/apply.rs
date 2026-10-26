use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::extraction::Rgb;

use super::lock_wallpaper;
use super::store::{read_json_optional, write_atomic};
use super::{Appearance, Selection, ThemeStore};

pub const TARGETS: &[&str] = &[
    "dojo", "niri", "kitty", "ghostty", "nvim", "opencode", "pi", "t3code", "yazi", "zen",
    "helium", "gtk", "qt",
];

#[derive(Debug, Serialize)]
pub struct TargetResult {
    pub target: String,
    pub status: String,
    pub detail: String,
}
#[derive(Debug, Serialize)]
pub struct ApplyReport {
    pub selection: Selection,
    pub targets: Vec<TargetResult>,
}

#[derive(Default, Deserialize, Serialize)]
struct Published {
    files: BTreeMap<String, String>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn publish(path: &Path, bytes: &[u8], registry: &mut Published) -> anyhow::Result<()> {
    let key = path.to_string_lossy().into_owned();
    if let Ok(info) = fs::symlink_metadata(path) {
        anyhow::ensure!(
            info.is_file() && !info.file_type().is_symlink(),
            "refusing to replace managed/non-regular path {}",
            path.display()
        );
        let current = fs::read(path)?;
        if current == bytes {
            registry.files.insert(key, digest(bytes));
            return Ok(());
        }
        anyhow::ensure!(
            registry.files.get(&key) == Some(&digest(&current)),
            "{} exists and is not unchanged Dojo output; preserve it and resolve ownership first",
            path.display()
        );
    }
    write_atomic(path, bytes)?;
    registry.files.insert(key, digest(bytes));
    Ok(())
}

fn publish_t3_theme(
    home: &Path,
    root: &Path,
    bytes: &[u8],
    registry: &mut Published,
) -> anyhow::Result<(&'static str, String)> {
    publish(&root.join("userdata/themes/dojo.json"), bytes, registry)?;
    let cli = home.join(".t3/bin/t3");
    if !cli.is_file() {
        return Ok((
            "needs_selection",
            "regular watched JSON published; select Dojo in T3 Code or install its bundled t3 CLI"
                .into(),
        ));
    }
    let result = command_with_timeout(
        cli.to_str().context("T3 CLI path is not UTF-8")?,
        &[
            "theme",
            "set",
            "dojo",
            "--base-dir",
            root.to_str().context("T3 home path is not UTF-8")?,
        ],
        Duration::from_secs(10),
    );
    match result {
        Ok(_) => Ok((
            "reload_requested",
            "regular watched JSON published; bundled t3 CLI selected Dojo for connected clients"
                .into(),
        )),
        Err(error) => Ok((
            "reload_failed",
            format!("theme JSON published, but selecting Dojo failed: {error:#}"),
        )),
    }
}

fn command(program: &str, args: &[&str]) -> anyhow::Result<String> {
    command_with_timeout(program, args, Duration::from_secs(3))
}

fn command_with_timeout(program: &str, args: &[&str], timeout: Duration) -> anyhow::Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {program}"))?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > timeout {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("{program} timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output()?;
    anyhow::ensure!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

pub fn system_appearance() -> anyhow::Result<Appearance> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?);
    let mut stream = UnixStream::connect(runtime.join("darkman/control.sock"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"get\n")?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    Appearance::parse(answer.trim())
}

pub fn set_system_appearance(appearance: Appearance) -> anyhow::Result<()> {
    command("darkman", &["set", appearance.name()])?;
    anyhow::ensure!(
        system_appearance()? == appearance,
        "darkman did not switch to {} mode",
        appearance.name()
    );
    Ok(())
}

fn signal_instances(name: &str, signal: &str) -> anyhow::Result<usize> {
    let instances = application_instances(name)?;
    for pid in &instances {
        command("kill", &[signal, &pid.to_string()])?;
    }
    Ok(instances.len())
}

fn application_instances(name: &str) -> anyhow::Result<Vec<u32>> {
    let uid = fs::metadata("/proc/self")?.uid();
    let mut instances = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|v| v.parse::<u32>().ok())
        else {
            continue;
        };
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.uid() != uid {
            continue;
        }
        let Ok(executable) = fs::read_link(path.join("exe")) else {
            continue;
        };
        if normalized_executable(&executable) != Some(name) {
            continue;
        }
        if name == "opencode" {
            let Ok(argv) = fs::read(path.join("cmdline")) else {
                continue;
            };
            let argument = argv.split(|b| *b == 0).nth(1).unwrap_or(&[]);
            let project = std::str::from_utf8(argument)
                .ok()
                .is_some_and(|arg| Path::new(arg).is_dir());
            if !argument.is_empty()
                && argument != b"attach"
                && !argument.starts_with(b"-")
                && !project
            {
                continue;
            }
        }
        instances.push(pid);
    }
    Ok(instances)
}

fn normalized_executable(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    Some(
        name.strip_prefix('.')
            .unwrap_or(name)
            .strip_suffix("-wrapped")
            .unwrap_or(name.strip_prefix('.').unwrap_or(name)),
    )
}

fn patch_json_settings(path: &Path, updates: serde_json::Value) -> anyhow::Result<()> {
    if let Ok(info) = fs::symlink_metadata(path) {
        anyhow::ensure!(
            info.is_file() && !info.file_type().is_symlink(),
            "{} is declaratively managed or non-regular; configure the theme declaratively",
            path.display()
        );
    }
    let mut value: serde_json::Value =
        read_json_optional(path)?.unwrap_or_else(|| serde_json::json!({}));
    let object = value
        .as_object_mut()
        .context("settings must be a JSON object (JSONC requires manual integration)")?;
    for (key, value) in updates.as_object().context("updates must be an object")? {
        object.insert(key.clone(), value.clone());
    }
    write_atomic(path, &serde_json::to_vec_pretty(&value)?)
}

pub fn apply(store: &ThemeStore, selection: Selection) -> anyhow::Result<ApplyReport> {
    let _lock = store.lock()?;
    apply_locked(store, selection)
}

pub fn refresh_target(store: &ThemeStore, target: &str) -> anyhow::Result<TargetResult> {
    anyhow::ensure!(TARGETS.contains(&target), "unknown theme target");
    let _lock = store.lock()?;
    let selection = store.selection()?.context("no selected theme")?;
    let theme = store.theme(&selection.generation)?;
    let outputs = theme.outputs(selection.appearance)?;
    let path = store.paths.root.join("published.json");
    let mut registry: Published = read_json_optional(&path)?.unwrap_or_default();
    let (status, detail) = publish_target(store, target, &selection, &outputs, &mut registry)?;
    write_atomic(&path, &serde_json::to_vec_pretty(&registry)?)?;
    Ok(TargetResult {
        target: target.into(),
        status: status.into(),
        detail,
    })
}

pub fn apply_wallpaper(
    store: &ThemeStore,
    path: &Path,
    bytes: Vec<u8>,
    fallback: Option<super::SemanticPalette>,
) -> anyhow::Result<ApplyReport> {
    publish_wallpaper(store, fallback, |theme| theme.with_wallpaper_snapshot(path, bytes))
}

/// Selects a generated wallpaper, keeping the palette, preset, appearance and targets as
/// [`apply_wallpaper`] does. `aspect` is the width over the height of the reference picture retained for palette
/// derivation and the lock and login screens, and `edge` its long side in pixels (see
/// [`crate::procedural::desktop::retained`]).
pub fn apply_procedural(
    store: &ThemeStore,
    recipe: crate::procedural::Recipe,
    aspect: f64,
    edge: u32,
    fallback: Option<super::SemanticPalette>,
) -> anyhow::Result<ApplyReport> {
    let reference = recipe.reference_png(aspect, edge)?;
    publish_wallpaper(store, fallback, |theme| theme.with_recipe(recipe, reference))
}

fn publish_wallpaper(
    store: &ThemeStore,
    fallback: Option<super::SemanticPalette>,
    retain: impl FnOnce(super::Theme) -> anyhow::Result<super::Theme>,
) -> anyhow::Result<ApplyReport> {
    let _lock = store.lock()?;
    let (theme, mut selection) = match store.selection()? {
        Some(selection) => (store.theme(&selection.generation)?, selection),
        None => {
            let palette = fallback
                .context("wallpaper needs the existing config palette when no theme is selected")?;
            let appearance = palette.appearance;
            (
                super::Theme::from_palette(palette)?,
                Selection {
                    generation: String::new(),
                    appearance,
                    follow_system: true,
                    targets: vec!["dojo".into()],
                },
            )
        }
    };
    let theme = retain(theme)?;
    selection.generation = store.generate(&theme)?;
    apply_locked(store, selection)
}

/// What [`refresh_lock_wallpaper`] did for the selected generation.
#[derive(Debug, Serialize)]
pub struct LockWallpaperReport {
    pub generation: String,
    pub appearance: String,
    /// The current activation's `lock-wallpaper.png`, `None` when the generation retains no wallpaper.
    pub path: Option<PathBuf>,
    /// The picture was rendered now rather than found current.
    pub rendered: bool,
    /// The activation gained the link now rather than already having it.
    pub linked: bool,
}

/// Brings the current activation's `lock-wallpaper.png` up to date for the selected
/// generation and appearance without applying anything: no target is published,
/// nothing is signalled, no new activation is made and `current` does not move.
/// For a theme applied before the lock wallpaper existed.
pub fn refresh_lock_wallpaper(store: &ThemeStore) -> anyhow::Result<LockWallpaperReport> {
    let _lock = store.lock()?;
    let selection = store.selection()?.context("no selected theme")?;
    let appearance = selection.appearance.name();
    let source = store
        .paths
        .root
        .join("generations")
        .join(&selection.generation)
        .join(appearance)
        .join(lock_wallpaper::FILE_NAME);
    let rendered = !lock_wallpaper::is_current(&source);
    ensure_lock_wallpaper(store, &selection.generation)?;
    let mut report = LockWallpaperReport {
        generation: selection.generation,
        appearance: appearance.into(),
        path: None,
        rendered: rendered && source.is_file(),
        linked: false,
    };
    if !source.is_file() {
        return Ok(report);
    }
    let active = store.paths.root.join("current");
    let activation = store.paths.root.join(
        fs::read_link(&active).with_context(|| format!("reading {}", active.display()))?,
    );
    let link = activation.join(lock_wallpaper::FILE_NAME);
    report.path = Some(link.clone());
    if fs::read_link(&link).is_ok_and(|target| target == source) {
        return Ok(report);
    }
    let staged = activation.join(format!(".lock-wallpaper-{}", std::process::id()));
    let _ = fs::remove_file(&staged);
    std::os::unix::fs::symlink(&source, &staged)?;
    fs::rename(&staged, &link)?;
    report.linked = true;
    Ok(report)
}

fn apply_locked(store: &ThemeStore, selection: Selection) -> anyhow::Result<ApplyReport> {
    for target in &selection.targets {
        anyhow::ensure!(
            TARGETS.contains(&target.as_str()),
            "unknown target {target}"
        );
    }
    let theme = store.theme(&selection.generation)?;
    let outputs = theme.outputs(selection.appearance)?;
    let registry_path = store.paths.root.join("published.json");
    let mut registry: Published = read_json_optional(&registry_path)?.unwrap_or_default();
    // Before commit, so the snapshot's symlinks include it. A failure is reported, not fatal.
    let lock_wallpaper = ensure_lock_wallpaper(store, &selection.generation).err();
    let mut results = Vec::new();
    for target in &selection.targets {
        let result = publish_target(store, target, &selection, &outputs, &mut registry);
        match result {
            Ok((status, detail)) => results.push(TargetResult {
                target: target.clone(),
                status: status.into(),
                detail,
            }),
            Err(error) => results.push(TargetResult {
                target: target.clone(),
                status: "blocked".into(),
                detail: format!("{error:#}"),
            }),
        }
        // Persist ownership after each target so partial publication is recoverable.
        write_atomic(&registry_path, &serde_json::to_vec_pretty(&registry)?)?;
    }
    store.commit(&selection)?;
    for target in &mut results {
        if target.target == "kitty" || target.target == "ghostty" || target.target == "opencode" {
            if target.status == "published" {
                let signal = if target.target == "kitty" {
                    "-USR1"
                } else {
                    "-USR2"
                };
                match signal_instances(&target.target, signal) {
                    Ok(count) => {
                        target.status = if count > 0 {
                            "reload_requested"
                        } else {
                            "ready"
                        }
                        .into();
                        target.detail =
                            format!("{}; signalled {count} existing UI processes", target.detail);
                    }
                    Err(error) => {
                        target.status = "reload_failed".into();
                        target.detail = format!("{error:#}");
                    }
                }
            }
        }
    }
    if let Some(error) = lock_wallpaper {
        results.push(TargetResult {
            target: "lock-wallpaper".into(),
            status: "blocked".into(),
            detail: format!("the lock screen falls back to a flat palette colour: {error:#}"),
        });
    }
    let report = ApplyReport {
        selection,
        targets: results,
    };
    write_atomic(
        &store.paths.root.join("last-apply.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

/// Makes `lock-wallpaper.png` exist in both appearance directories of a generation
/// exactly when the generation retained a wallpaper, rendered for that appearance's
/// text colour by the current scrim rule. Idempotent: a file the current rule
/// already rendered is kept, so appearance switches stay cheap, while one from an
/// older rule is rendered again.
fn ensure_lock_wallpaper(store: &ThemeStore, generation: &str) -> anyhow::Result<()> {
    let directory = store.paths.root.join("generations").join(generation);
    let targets = [Appearance::Dark, Appearance::Light].map(|appearance| {
        (
            appearance,
            directory
                .join(appearance.name())
                .join(lock_wallpaper::FILE_NAME),
        )
    });
    let wallpaper = match fs::read(directory.join("wallpaper")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            for (_, path) in &targets {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| format!("removing {}", path.display()));
                    }
                }
            }
            return Ok(());
        }
        Err(error) => return Err(error).context("reading the retained wallpaper"),
    };
    let stale: Vec<_> = targets
        .iter()
        .filter(|(_, path)| !lock_wallpaper::is_current(path))
        .collect();
    if stale.is_empty() {
        return Ok(());
    }
    let theme = store.theme(generation)?;
    let picture = lock_wallpaper::blur(&lock_wallpaper::decode(&wallpaper)?);
    let mut encoded: Vec<(Rgb, Vec<u8>)> = Vec::new();
    for (appearance, path) in stale {
        // The hint line is drawn in the muted colour, the weakest text on the wallpaper.
        let text = Rgb::parse(theme.palette(*appearance).color("foreground_muted"))?;
        let png = match encoded.iter().find(|(known, _)| *known == text) {
            Some((_, png)) => png,
            None => {
                encoded.push((text, lock_wallpaper::encode(&picture.scrimmed(text))?));
                &encoded[encoded.len() - 1].1
            }
        };
        write_atomic(path, png)?;
    }
    Ok(())
}

fn publish_target(
    store: &ThemeStore,
    target: &str,
    selection: &Selection,
    outputs: &BTreeMap<String, Vec<u8>>,
    registry: &mut Published,
) -> anyhow::Result<(&'static str, String)> {
    let paths = &store.paths;
    let current = paths.root.join("current");
    let put = |name: &str, path: &Path, registry: &mut Published| -> anyhow::Result<()> {
        publish(
            path,
            outputs
                .get(name)
                .with_context(|| format!("missing exporter {name}"))?,
            registry,
        )
    };
    match target {
        "dojo"=>Ok(("published","running surfaces observe the selected palette; user config is untouched".into())),
        "niri"=>{
            let path = paths.root.join("niri.kdl");
            put("niri.kdl",&path,registry)?;
            command("niri",&["msg","action","load-config-file"])?;
            Ok(("reload_requested","Niri reloaded the Dojo background, workspace-swipe backdrop, focus-ring and agent cursor colors from the active theme".into()))
        },
        "kitty"=>{
            let config=fs::read_to_string(paths.config.join("kitty/kitty.conf"))?;
            anyhow::ensure!(config.contains(&current.join("kitty.conf").to_string_lossy().to_string()),"Kitty needs the Dojo include from Home Manager before reload");
            Ok(("published","included color fragment selected".into()))
        },
        "ghostty"=>{
            let config=fs::read_to_string(paths.config.join("ghostty/config"))?;
            anyhow::ensure!(config.contains(&current.join("ghostty.conf").to_string_lossy().to_string()),"Ghostty needs the Dojo config-file include from Home Manager before reload");
            Ok(("published","included color fragment selected".into()))
        },
        "nvim"=>Ok(("published","dedicated Dojo colorscheme reads current/nvim.json; configured instances reload in-process, or :colorscheme dojo".into())),
        "opencode"=>{
            put("opencode.json",&paths.config.join("opencode/themes/dojo.json"),registry)?;
            patch_json_settings(&paths.config.join("opencode/cli.json"),serde_json::json!({"theme":{"name":"dojo","mode":selection.appearance.name()}}))?;
            Ok(("published","paired custom theme selected in cli.json; server processes are not signalled".into()))
        },
        "pi"=>{
            let agent=std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from).unwrap_or_else(||paths.home.join(".pi/agent"));
            put("pi.json",&agent.join("themes/dojo.json"),registry)?;
            patch_json_settings(&agent.join("settings.json"),serde_json::json!({"theme":"dojo"}))?;
            Ok(("needs_selection","theme is installed; existing Pi sessions must select dojo once; active user-theme files then hot-reload".into()))
        },
        "t3code"=>{
            let root=std::env::var_os("T3CODE_HOME").map(PathBuf::from).unwrap_or_else(||paths.home.join(".t3"));
            publish_t3_theme(&paths.home,&root,outputs.get("t3code.json").context("missing T3 exporter")?,registry)
        },
        "yazi"=>{
            let root=paths.config.join("yazi/flavors/dojo.yazi");
            put("yazi.toml",&root.join("flavor.toml"),registry)?;
            put("tmtheme.xml",&root.join("tmtheme.xml"),registry)?;
            put_bytes(&root.join("README.md"),b"# Dojo\nGenerated local flavor.\n",registry)?;
            let init=fs::read_to_string(paths.config.join("yazi/init.lua")).unwrap_or_default();
            anyhow::ensure!(init.contains("dojo-theme"),"Yazi needs the stable dojo-theme DDS plugin/init and flavor selection from Home Manager");
            if application_instances("yazi")?.is_empty() {
                Ok(("ready","flavor, syntax palette and stable plugin provisioned; no running Yazi instance needs a DDS reload".into()))
            } else {
                command("ya",&["pub-to","0","dojo-theme","--str","reload"])?;
                Ok(("reload_requested","flavor and syntax preview theme published; DDS reload sent".into()))
            }
        },
        "zen"=>{
            let runtime=PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR unavailable")?);
            let ack=runtime.join("dojo-zen-ack.json");
            let value:Option<serde_json::Value>=read_json_optional(&ack)?;
            Ok((if value.is_some() {"published"} else {"needs_setup"},"browser.theme.update payload available at current/zen.json; extension/native host must be installed once; last acknowledgement is not proof of this apply".into()))
        },
        "helium"=>{
            let palette: super::SemanticPalette = serde_json::from_slice(
                outputs.get("palette.json").context("missing semantic palette")?
            )?;
            command("dojo-chromium-theme-policy", &[palette.color("background")])?;
            if application_instances("helium")?.is_empty() {
                Ok(("ready", "Built-in browser theme policy updated from the background color; Helium will apply it on launch. Chrome derives the remaining colors.".into()))
            } else {
                command("helium", &["--refresh-platform-policy", "--no-startup-window"])?;
                Ok(("reload_requested", "Built-in browser theme policy updated and immediate policy refresh requested; Chrome derives the remaining colors.".into()))
            }
        },
        "gtk"=>{
            let name=format!("dojo-{}-{}",selection.generation,selection.appearance.name());
            let root=paths.data.join("themes").join(&name);
            put("gtk-3.0.css",&root.join("gtk-3.0/gtk.css"),registry)?;
            put("gtk-4.0.css",&root.join("gtk-4.0/gtk.css"),registry)?;
            command("gsettings",&["set","org.gnome.desktop.interface","gtk-theme",&name])?;
            command("gsettings",&["set","org.gnome.desktop.interface","color-scheme",if selection.appearance==Appearance::Dark {"prefer-dark"} else {"prefer-light"}])?;
            Ok(("reload_requested","new named GTK theme selected; libadwaita full palettes and sandbox/portal routing are not certified".into()))
        },
        "qt"=>{
            let mut count=0;
            for version in [5,6] {
                let directory=paths.config.join(format!("qt{version}ct"));
                let config=directory.join(format!("qt{version}ct.conf"));
                let expected=current.join(format!("qt{version}.conf")).to_string_lossy().into_owned();
                if let Ok(contents)=fs::read_to_string(&config) {
                    anyhow::ensure!(contents.contains(&expected)&&contents.contains("custom_palette=true"),"{} needs the Dojo custom palette integration",config.display());
                    put_bytes(&directory.join(".dojo-generation"),format!("{} {}\n",selection.generation,selection.appearance.name()).as_bytes(),registry)?;
                    count+=1;
                }
            }
            anyhow::ensure!(count>0,"qtct plugins/configuration need one-time provisioning; existing apps must have loaded the platform plugin");
            Ok(("reload_requested",format!("triggered {count} qtct directory watchers; Widgets palette reload is debounced, QML/app-owned styles are not certified")))
        },
        _=>anyhow::bail!("unknown theme target {target}"),
    }
}

fn put_bytes(path: &Path, bytes: &[u8], registry: &mut Published) -> anyhow::Result<()> {
    publish(path, bytes, registry)
}

pub fn refresh_system(store: &ThemeStore) -> anyhow::Result<Option<ApplyReport>> {
    let _lock = store.lock()?;
    let Some(mut selection) = store.selection()? else {
        return Ok(None);
    };
    if !selection.follow_system {
        return Ok(None);
    }
    let appearance = system_appearance()?;
    if appearance == selection.appearance {
        return Ok(None);
    }
    selection.appearance = appearance;
    Ok(Some(apply_locked(store, selection)?))
}

pub fn watch(store: &ThemeStore) -> anyhow::Result<()> {
    loop {
        match refresh_system(store) {
            Ok(Some(report)) => println!("{}", serde_json::to_string(&report)?),
            Ok(None) => {}
            Err(error) => eprintln!("Dojo theme watcher kept its current selection: {error:#}"),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t3_publication_selects_the_theme_in_the_configured_home() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        use std::time::{SystemTime, UNIX_EPOCH};
        let home = std::env::temp_dir().join(format!(
            "dojo-t3-selection-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let root = home.join("custom-t3-home");
        let cli = home.join(".t3/bin/t3");
        fs::create_dir_all(cli.parent().context("CLI directory")?)?;
        fs::write(
            &cli,
            "#!/bin/sh\nset -eu\n[ \"$1\" = theme ]\n[ \"$2\" = set ]\n[ \"$4\" = --base-dir ]\n[ -f \"$5/userdata/themes/$3.json\" ]\n[ ! -L \"$5/userdata/themes/$3.json\" ]\nprintf '%s' \"$3\" > \"$5/selected-theme\"\n",
        )?;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755))?;
        let bytes = br##"{"name":"Dojo","appearance":"dark","canvas":"#1e1e2e"}"##;
        let mut registry = Published::default();
        let (status, _) = publish_t3_theme(&home, &root, bytes, &mut registry)?;
        assert_eq!(status, "reload_requested");
        assert_eq!(fs::read_to_string(root.join("selected-theme"))?, "dojo");
        assert_eq!(fs::read(root.join("userdata/themes/dojo.json"))?, bytes);
        fs::write(&cli, "#!/bin/sh\nexit 1\n")?;
        let (status, detail) = publish_t3_theme(&home, &root, bytes, &mut registry)?;
        assert_eq!(status, "reload_failed");
        assert!(detail.contains("theme JSON published"));
        fs::remove_file(cli)?;
        let (status, _) = publish_t3_theme(&home, &root, bytes, &mut registry)?;
        assert_eq!(status, "needs_selection");
        fs::remove_dir_all(home)?;
        Ok(())
    }

    fn test_store(root: &Path) -> ThemeStore {
        ThemeStore::new(super::super::Paths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            root: root.join("themes"),
        })
    }

    fn gradient_png(width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x * 255 / width) as u8, (y * 255 / height) as u8, 90, 255])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png)?;
        Ok(png.into_inner())
    }

    fn dojo_selection(generation: String, appearance: Appearance) -> Selection {
        Selection {
            generation,
            appearance,
            follow_system: false,
            targets: vec!["dojo".into()],
        }
    }

    fn lock_wallpaper_in_current(store: &ThemeStore) -> PathBuf {
        store
            .paths
            .root
            .join("current")
            .join(lock_wallpaper::FILE_NAME)
    }

    #[test]
    fn refreshing_the_lock_wallpaper_repairs_the_current_activation_and_nothing_else() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = test_store(dir.path());
        let fallback = crate::theme::presets()?[0].palette.clone();
        let report = apply_wallpaper(
            &store,
            &dir.path().join("wall.png"),
            gradient_png(1920, 1080)?,
            Some(fallback),
        )?;
        let root = &store.paths.root;
        let current = lock_wallpaper_in_current(&store);
        let rendered = fs::read(&current)?;
        let activation = fs::read_link(root.join("current"))?;
        let palette_target = fs::read_link(root.join("current/palette.json"))?;

        // The state a theme applied before the lock wallpaper existed is in.
        let generation = root.join("generations").join(&report.selection.generation);
        for appearance in ["dark", "light"] {
            fs::remove_file(generation.join(appearance).join(lock_wallpaper::FILE_NAME))?;
        }
        fs::remove_file(&current)?;
        let published = fs::read(root.join("published.json"))?;

        let first = refresh_lock_wallpaper(&store)?;
        assert!(first.rendered && first.linked);
        assert_eq!(first.path.as_deref(), Some(fs::canonicalize(root.join("current"))?.join(lock_wallpaper::FILE_NAME).as_path()));
        assert_eq!(fs::read(&current)?, rendered);
        assert_eq!(fs::read_link(root.join("current"))?, activation, "no new activation");
        assert_eq!(fs::read_link(root.join("current/palette.json"))?, palette_target);
        assert_eq!(fs::read(root.join("published.json"))?, published, "nothing was published");
        assert_eq!(fs::read_dir(root.join("activations"))?.count(), 1);

        let again = refresh_lock_wallpaper(&store)?;
        assert!(!again.rendered && !again.linked);
        assert_eq!(fs::read(&current)?, rendered);
        Ok(())
    }

    #[test]
    fn refreshing_without_a_retained_wallpaper_writes_nothing() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = test_store(dir.path());
        assert!(refresh_lock_wallpaper(&store).is_err(), "no theme is selected");
        let preset = &crate::theme::presets()?[0];
        let theme = super::super::Theme::from_preset(preset, &store)?;
        let generation = store.generate(&theme)?;
        apply(&store, dojo_selection(generation, preset.appearance))?;
        let report = refresh_lock_wallpaper(&store)?;
        assert_eq!(report.path, None);
        assert!(!report.rendered && !report.linked);
        assert!(fs::symlink_metadata(lock_wallpaper_in_current(&store)).is_err());
        Ok(())
    }

    #[test]
    fn lock_wallpaper_follows_the_wallpaper_snapshot_and_never_leaks() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = test_store(dir.path());
        let fallback = crate::theme::presets()?[0].palette.clone();
        let current = lock_wallpaper_in_current(&store);

        let report = apply_wallpaper(
            &store,
            &dir.path().join("wall.png"),
            gradient_png(1920, 1080)?,
            Some(fallback.clone()),
        )?;
        assert_eq!(report.targets.len(), 1, "success adds no extra result");
        let png = image::open(&current)?;
        assert_eq!((png.width(), png.height()), (480, 270));
        assert!(store.paths.root.join("current/palette.json").is_file());
        let generation = report.selection.generation.clone();
        let light_copy = store
            .paths
            .root
            .join("generations")
            .join(&generation)
            .join("light")
            .join(lock_wallpaper::FILE_NAME);
        assert!(light_copy.is_file());
        let dark_bytes = fs::read(&current)?;

        let light = apply(
            &store,
            dojo_selection(generation.clone(), Appearance::Light),
        )?;
        assert_eq!(light.targets.len(), 1);
        assert_eq!(
            fs::read(&current)?,
            dark_bytes,
            "appearance switch keeps the same image"
        );

        // A stale file planted in a generation without a wallpaper is removed, not linked.
        let bare = store.generate(&super::super::Theme::from_palette(fallback.clone())?)?;
        let planted = store
            .paths
            .root
            .join("generations")
            .join(&bare)
            .join("dark")
            .join(lock_wallpaper::FILE_NAME);
        fs::write(&planted, b"stale")?;
        let report = apply(&store, dojo_selection(bare, Appearance::Dark))?;
        assert_eq!(report.targets.len(), 1);
        assert!(
            fs::symlink_metadata(&current).is_err(),
            "no wallpaper, no lock image"
        );
        assert!(!planted.exists());
        assert!(store.paths.root.join("current/palette.json").is_file());

        // A preset applied with no wallpaper retained writes nothing either.
        let fresh = tempfile::tempdir()?;
        let fresh_store = test_store(fresh.path());
        let preset = &crate::theme::presets()?[0];
        let theme = super::super::Theme::from_preset(preset, &fresh_store)?;
        let generation = fresh_store.generate(&theme)?;
        apply(&fresh_store, dojo_selection(generation, preset.appearance))?;
        assert!(fs::symlink_metadata(lock_wallpaper_in_current(&fresh_store)).is_err());

        // Re-selecting the wallpaper generation restores the image.
        let report = apply(
            &store,
            dojo_selection(light.selection.generation, Appearance::Dark),
        )?;
        assert_eq!(report.targets.len(), 1);
        assert_eq!(fs::read(&current)?, dark_bytes);
        Ok(())
    }

    fn two_palette_generation(store: &ThemeStore, wallpaper: Vec<u8>) -> anyhow::Result<String> {
        let presets = crate::theme::presets()?;
        let pick = |appearance| {
            presets
                .iter()
                .find(|preset| preset.appearance == appearance)
                .map(|preset| preset.palette.clone())
                .context("preset")
        };
        let mut theme = super::super::Theme::from_palette(pick(Appearance::Dark)?)?;
        theme.light = pick(Appearance::Light)?;
        let theme = theme.with_wallpaper_snapshot(Path::new("/wall.png"), wallpaper)?;
        store.generate(&theme)
    }

    fn generation_file(store: &ThemeStore, generation: &str, appearance: Appearance) -> PathBuf {
        store
            .paths
            .root
            .join("generations")
            .join(generation)
            .join(appearance.name())
            .join(lock_wallpaper::FILE_NAME)
    }

    #[test]
    fn each_appearance_gets_a_wallpaper_scrimmed_for_its_own_text() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = test_store(dir.path());
        let generation = two_palette_generation(&store, gradient_png(1920, 1080)?)?;

        apply(
            &store,
            dojo_selection(generation.clone(), Appearance::Light),
        )?;

        let dark = fs::read(generation_file(&store, &generation, Appearance::Dark))?;
        let light = fs::read(generation_file(&store, &generation, Appearance::Light))?;
        assert_ne!(dark, light, "different palettes, different scrims");
        let head = |bytes: &[u8]| {
            String::from_utf8_lossy(&bytes[..lock_wallpaper::MARKER_PREFIX]).into_owned()
        };
        assert!(head(&dark).contains("scrim=Dark:"), "{}", head(&dark));
        assert!(head(&light).contains("scrim=Light:"), "{}", head(&light));
        assert_eq!(fs::read(lock_wallpaper_in_current(&store))?, light);
        assert!(lock_wallpaper::is_current(&generation_file(
            &store,
            &generation,
            Appearance::Dark
        )));
        Ok(())
    }

    #[test]
    fn a_wallpaper_from_an_older_rule_is_rendered_again_and_a_current_one_is_kept()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = test_store(dir.path());
        let generation = two_palette_generation(&store, gradient_png(1920, 1080)?)?;
        apply(&store, dojo_selection(generation.clone(), Appearance::Dark))?;
        let dark_path = generation_file(&store, &generation, Appearance::Dark);
        let light_path = generation_file(&store, &generation, Appearance::Light);
        let dark = fs::read(&dark_path)?;
        let light = fs::read(&light_path)?;

        // The light file predates the rule (a plain PNG, as the constant 45% dim wrote).
        let mut plain = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(480, 270, image::Rgba([40, 40, 40, 255]))
            .write_to(&mut plain, image::ImageFormat::Png)?;
        fs::write(&light_path, plain.get_ref())?;
        let kept = fs::metadata(&dark_path)?.ino();

        apply(&store, dojo_selection(generation.clone(), Appearance::Dark))?;
        assert_eq!(fs::read(&light_path)?, light, "the old file was replaced");
        assert_eq!(fs::read(&dark_path)?, dark);
        assert_eq!(
            fs::metadata(&dark_path)?.ino(),
            kept,
            "a current file is not rewritten"
        );

        // A file stamped with an earlier rule number is replaced too.
        let mut older = dark.clone();
        let at = older
            .windows(6)
            .position(|w| w == b"rule=2")
            .context("rule marker")?;
        older[at + 5] = b'1';
        fs::write(&dark_path, &older)?;
        apply(&store, dojo_selection(generation, Appearance::Dark))?;
        assert_eq!(fs::read(&dark_path)?, dark);
        Ok(())
    }

    #[test]
    fn a_failed_lock_wallpaper_render_is_reported_and_does_not_fail_the_apply() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        let store = test_store(dir.path());
        let fallback = crate::theme::presets()?[0].palette.clone();
        let mut theme = super::super::Theme::from_palette(fallback)?;
        theme.image_bytes = b"retained bytes that are not an image".to_vec();
        let generation = store.generate(&theme)?;

        let report = apply(&store, dojo_selection(generation.clone(), Appearance::Dark))?;

        assert_eq!(store.selection()?.map(|s| s.generation), Some(generation));
        assert!(store.paths.root.join("current/palette.json").is_file());
        assert!(fs::symlink_metadata(lock_wallpaper_in_current(&store)).is_err());
        assert_eq!(report.targets[0].target, "dojo");
        assert_eq!(report.targets[0].status, "published");
        let failure = report.targets.last().context("results")?;
        assert_eq!(failure.target, "lock-wallpaper");
        assert_eq!(failure.status, "blocked");
        assert!(
            failure.detail.contains("decoding wallpaper"),
            "{}",
            failure.detail
        );
        let recorded: serde_json::Value =
            serde_json::from_slice(&fs::read(store.paths.root.join("last-apply.json"))?)?;
        assert_eq!(recorded["targets"][1]["target"], "lock-wallpaper");
        assert_eq!(recorded["targets"][1]["status"], "blocked");
        Ok(())
    }

    #[test]
    fn reload_identifies_nix_wrapped_executables_without_comm_truncation() {
        assert_eq!(
            normalized_executable(Path::new("/nix/store/pkg/bin/.opencode-wrapped")),
            Some("opencode")
        );
        assert_eq!(
            normalized_executable(Path::new("/nix/store/pkg/bin/.kitty-wrapped")),
            Some("kitty")
        );
        assert_eq!(
            normalized_executable(Path::new("/usr/bin/ghostty")),
            Some("ghostty")
        );
    }
    #[test]
    fn unowned_output_and_symlinks_are_never_overwritten() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "dojo-publish-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&root)?;
        let path = root.join("theme.json");
        let mut registry = Published::default();
        fs::write(&path, b"user file")?;
        assert!(publish(&path, b"generated", &mut registry).is_err());
        assert_eq!(fs::read(&path)?, b"user file");
        fs::remove_file(&path)?;
        publish(&path, b"first", &mut registry)?;
        publish(&path, b"second", &mut registry)?;
        assert_eq!(fs::read(&path)?, b"second");
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&path, &alias)?;
        assert!(publish(&alias, b"overwrite", &mut registry).is_err());
        fs::remove_file(alias)?;
        fs::remove_file(path)?;
        fs::remove_dir(root)?;
        Ok(())
    }
}
