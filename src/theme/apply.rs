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

use super::store::{read_json_optional, write_atomic};
use super::{Appearance, Selection, ThemeStore};

pub const TARGETS: &[&str] = &[
    "dojo", "niri", "kitty", "ghostty", "nvim", "opencode", "pi", "t3code", "zed", "yazi", "zen",
    "helium", "gtk", "qt", "delta",
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

pub(super) fn set_system_appearance(appearance: Appearance) -> anyhow::Result<()> {
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

fn merge_theme_values(current: &mut serde_json::Value, generated: &serde_json::Value) {
    if let (Some(current), Some(generated)) = (current.as_object_mut(), generated.as_object()) {
        for (key, value) in generated {
            merge_theme_values(
                current
                    .entry(key.clone())
                    .or_insert(serde_json::Value::Null),
                value,
            );
        }
    } else {
        *current = generated.clone();
    }
}

fn patch_zed_settings(
    store: &ThemeStore,
    selection: &Selection,
    outputs: &BTreeMap<String, Vec<u8>>,
) -> anyhow::Result<()> {
    let path = store.paths.config.join("zed/settings.json");
    if let Ok(info) = fs::symlink_metadata(&path) {
        anyhow::ensure!(
            info.is_file() && !info.file_type().is_symlink(),
            "{} is declaratively managed or non-regular",
            path.display()
        );
    }
    let mut settings: serde_json::Value =
        read_json_optional(&path)?.unwrap_or_else(|| serde_json::json!({}));
    let object = settings
        .as_object_mut()
        .context("Zed settings must be a JSON object; JSONC requires deliberate integration")?;
    let overrides = object
        .entry("theme_overrides")
        .or_insert(serde_json::json!({}));
    let old: Option<serde_json::Value> =
        read_json_optional(&store.paths.root.join("current/zed-overrides.json"))?;
    // Migrate only our previous flat output, never an arbitrary user's overrides.
    if old.as_ref() == Some(&*overrides) {
        *overrides = serde_json::json!({});
    }
    let named = overrides
        .as_object_mut()
        .context("Zed theme_overrides must be an object")?;
    anyhow::ensure!(
        named.values().all(serde_json::Value::is_object),
        "legacy flat Zed overrides are not unchanged Dojo output; preserve and migrate them manually"
    );
    let family: serde_json::Value = serde_json::from_slice(&outputs["zed.json"])?;
    for theme in family["themes"]
        .as_array()
        .context("Zed theme family missing")?
    {
        let name = theme["name"].as_str().context("Zed theme name missing")?;
        merge_theme_values(
            named
                .entry(name.to_string())
                .or_insert(serde_json::json!({})),
            &theme["style"],
        );
    }
    object.insert(
        "theme".into(),
        serde_json::json!(if selection.appearance == Appearance::Dark {
            "Dojo Dark"
        } else {
            "Dojo Light"
        }),
    );
    write_atomic(&path, &serde_json::to_vec_pretty(&settings)?)
}

pub fn apply(store: &ThemeStore, selection: Selection) -> anyhow::Result<ApplyReport> {
    let _lock = store.lock()?;
    apply_locked(store, selection)
}

pub fn apply_wallpaper(
    store: &ThemeStore,
    path: &Path,
    bytes: Vec<u8>,
    fallback: Option<super::SemanticPalette>,
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
    let theme = theme.with_wallpaper_snapshot(path, bytes)?;
    selection.generation = store.generate(&theme)?;
    apply_locked(store, selection)
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
            Ok(("reload_requested","Niri reloaded the Dojo focus-ring colors from the active theme".into()))
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
        "zed"=>{
            put("zed.json",&paths.config.join("zed/themes/dojo.json"),registry)?;
            patch_zed_settings(store,selection,outputs)?;
            let running=!application_instances("zed-editor")?.is_empty()||!application_instances("zed")?.is_empty();
            Ok((if running {"reload_requested"} else {"ready"},"theme-name-keyed overrides published; Zed observes settings changes and refreshes windows without restart".into()))
        },
        "delta"=>Ok(("unsupported","Zed Delta uses synced account settings and an encrypted local cache. No verified external custom-palette/live-reload interface is available; its cache/database are never modified.".into())),
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
            let policy: serde_json::Value = serde_json::from_slice(
                outputs.get("helium.json").context("missing Helium policy exporter")?,
            )?;
            let color = policy
                .get("BrowserThemeColor")
                .and_then(serde_json::Value::as_str)
                .context("missing Helium BrowserThemeColor")?;
            command("dojo-chromium-theme-policy", &[color])?;
            Ok(("published","Chromium managed BrowserThemeColor policy updated; Helium watches the shared policy directory and should update within about five seconds. Chromium derives the chrome color from this seed; some internal pages may retain stock colors.".into()))
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

    #[test]
    fn zed_live_overrides_are_named_and_preserve_unrelated_settings() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "dojo-zed-overrides-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let store = ThemeStore::new(super::super::Paths {
            home: root.clone(),
            config: root.join("config"),
            data: root.join("data"),
            root: root.join("themes"),
        });
        let settings = store.paths.config.join("zed/settings.json");
        fs::create_dir_all(settings.parent().context("settings directory")?)?;
        let other = serde_json::json!({"editor.background":"#112233"});
        write_atomic(
            &settings,
            &serde_json::to_vec(
                &serde_json::json!({"ui_font_size":16,"theme_overrides":{"Other Theme":other,"Dojo Dark":{"syntax":{"number":{"font_style":"italic","color":"#112233"}}}}}),
            )?,
        )?;
        let catalog = super::super::presets()?;
        let preset = catalog
            .iter()
            .find(|preset| preset.id == "tokyo-night-night")
            .context("Night")?;
        let theme = super::super::Theme::from_preset(preset, &store)?;
        let generation = store.generate(&theme)?;
        let report = apply(
            &store,
            Selection {
                generation,
                appearance: Appearance::Dark,
                follow_system: false,
                targets: vec!["zed".into()],
            },
        )?;
        assert!(matches!(
            report.targets[0].status.as_str(),
            "reload_requested" | "ready"
        ));
        let actual: serde_json::Value = serde_json::from_slice(&fs::read(&settings)?)?;
        assert_eq!(actual["ui_font_size"], 16);
        assert_eq!(actual["theme_overrides"]["Other Theme"], other);
        assert_eq!(
            actual["theme_overrides"]["Dojo Dark"]["editor.background"],
            "#1a1b26"
        );
        assert_eq!(
            actual["theme_overrides"]["Dojo Dark"]["syntax"]["number"]["color"],
            "#ff9e64"
        );
        assert_eq!(
            actual["theme_overrides"]["Dojo Dark"]["syntax"]["number"]["font_style"],
            "italic"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn only_unchanged_dojo_flat_zed_overrides_are_migrated() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "dojo-zed-migration-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let store = ThemeStore::new(super::super::Paths {
            home: root.clone(),
            config: root.join("config"),
            data: root.join("data"),
            root: root.join("themes"),
        });
        let catalog = super::super::presets()?;
        let theme = super::super::Theme::from_preset(&catalog[0], &store)?;
        let generation = store.generate(&theme)?;
        store.commit(&Selection {
            generation: generation.clone(),
            appearance: Appearance::Light,
            follow_system: false,
            targets: vec!["dojo".into()],
        })?;
        let flat: serde_json::Value =
            serde_json::from_slice(&theme.outputs(Appearance::Light)?["zed-overrides.json"])?;
        let settings = store.paths.config.join("zed/settings.json");
        fs::create_dir_all(settings.parent().context("settings directory")?)?;
        write_atomic(
            &settings,
            &serde_json::to_vec(&serde_json::json!({"theme_overrides":flat}))?,
        )?;
        let report = apply(
            &store,
            Selection {
                generation: generation.clone(),
                appearance: Appearance::Light,
                follow_system: false,
                targets: vec!["zed".into()],
            },
        )?;
        assert!(matches!(
            report.targets[0].status.as_str(),
            "reload_requested" | "ready"
        ));
        let updated: serde_json::Value = serde_json::from_slice(&fs::read(&settings)?)?;
        assert!(updated["theme_overrides"]["Dojo Light"].is_object());
        let user = serde_json::json!({"theme_overrides":{"editor.background":"#123456"}});
        write_atomic(&settings, &serde_json::to_vec(&user)?)?;
        let report = apply(
            &store,
            Selection {
                generation,
                appearance: Appearance::Light,
                follow_system: false,
                targets: vec!["zed".into()],
            },
        )?;
        assert_eq!(report.targets[0].status, "blocked");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&settings)?)?,
            user
        );
        fs::remove_dir_all(root)?;
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
