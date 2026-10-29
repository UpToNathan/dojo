//! Links libghostty-vt statically (nixpkgs `libghostty-vt`, found through pkg-config) and
//! generates its bindings, plus a table from W3C key codes to `GhosttyKey`.
use std::{env, fs, path::PathBuf};

fn main() {
    let library = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("libghostty-vt-static")
        .expect("libghostty-vt-static not found by pkg-config; run Cargo through `devenv shell --`");
    // The static .pc names the archive by path; link it by name from its directory instead.
    let archive = library
        .link_files
        .first()
        .cloned()
        .or_else(|| {
            library.libs.is_empty().then_some(())?;
            None
        })
        .unwrap_or_else(|| {
            library.link_paths.first().expect("libghostty-vt link path").join("libghostty-vt.a")
        });
    let dir = archive.parent().expect("archive directory");
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    let include = library.include_paths.first().expect("libghostty-vt include path").clone();
    let bindings = bindgen::Builder::default()
        .header(include.join("ghostty/vt.h").display().to_string())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_function("ghostty_.*")
        .allowlist_type("Ghostty.*")
        .allowlist_var("GHOSTTY_.*")
        .prepend_enum_name(false)
        .derive_default(true)
        .layout_tests(false)
        .generate()
        .expect("libghostty-vt bindings");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let text = bindings.to_string();
    fs::write(out.join("ghostty.rs"), &text).unwrap();

    // `GHOSTTY_KEY_ARROW_UP` is W3C `ArrowUp`; single letters are `KeyA`.
    let mut table = String::from("pub fn key_from_code(code: &str) -> Option<GhosttyKey> {\n    Some(match code {\n");
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("pub const GHOSTTY_KEY_") else { continue };
        let Some((name, _)) = rest.split_once(':') else { continue };
        if !line.contains(": GhosttyKey =") || name.starts_with("ACTION") || name == "MAX_VALUE" || name == "UNIDENTIFIED" {
            continue;
        }
        let mut code: String = name
            .split('_')
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map(|c| c.to_string() + &chars.as_str().to_lowercase()).unwrap_or_default()
            })
            .collect();
        if name.len() == 1 {
            code = format!("Key{name}");
        }
        table.push_str(&format!("        {code:?} => GHOSTTY_KEY_{name},\n"));
    }
    table.push_str("        _ => return None,\n    })\n}\n");
    fs::write(out.join("keys.rs"), table).unwrap();
}
