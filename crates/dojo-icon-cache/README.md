# Dojo icon cache

This crate ports the complete pinned CUA icon cache and its tests behind an explicitly owned directory. It retains normalized 32/64-pixel PNGs, memory and disk eviction, single-flight fetches, batch cache lookups, negative results, and transient memory-only previews. It removes `cua-home`, implicit default directories, and process-wide cache instances.

`LinuxIcons` resolves local desktop icons with the existing `freedesktop-icons` lookup. The host supplies permitted source roots and a separate cache directory. Source files must be regular, stay within those canonical roots, and match the opened descriptor's identity. Reads are nonblocking and bounded to 4 MiB. PNG decoding limits each dimension to 4096 and decoded allocation to 64 MiB. There is no network fetcher.

## Linux adapter

Construct one `LinuxIcons` owner for the application and call `get` or `get_many` from the launcher's background executor. `get_many` returns an independent result for each request, so one corrupt icon cannot suppress valid rows. The cache's underlying coherent batch-fetch API also remains available.

```rust,ignore
let icons = LinuxIcons::new(cache_root, Config::default(), trusted_icon_roots)?;
let request = IconRequest {
    app_id: app.id.clone(),
    icon_name: icon_name.clone(),
    scale,
};
let asset = icons.get(&request).await?;
```

`IconAsset::Png` contains the reusable normalized PNG bytes. `OriginalPath` preserves SVG and other existing GPUI formats. PNG requests at scale 3 or 4 also keep the original file path, because the fixed 64-pixel CUA format would reduce Dojo's current resolution. SVG rendering therefore continues through `gpui::img(path)`; it never becomes a missing PNG.

Cache keys include desktop identity, theme, requested scale, original icon name, canonical source path, and file identity/revision. Changed source files receive new keys. Missing results have a TTL, including across process restarts. Invalid, oversized, untrusted, or failed reads return errors and do not become remembered missing icons.

## Launcher integration scope

Current `src/launcher/icons.rs` already uses `freedesktop-icons`' positive lookup cache. `src/launcher.rs` stores paths and missing results by catalogue index for the lifetime of each launcher window, and GPUI owns image rendering. Those behaviors remain useful. The new cache adds persistent normalized PNG data, bounded retained entries, a missing-result TTL, source revisions, and cancellation-safe shared fetches across launcher/catalogue lifetimes. No measured speedup is claimed.

During integration, replace `icons::resolve` with this adapter. Remove the old per-window map as an authoritative lookup or permanent missing-icon cache. A row may retain a GPUI image handle as display state; it must not duplicate cache policy or prevent a missing result from being retried after the cache TTL. Share one owner across launcher windows instead of constructing two caches for the same source.

The crate deliberately does not replace freedesktop theme discovery or GPUI's SVG/other-format renderer. Trusted source roots must include the canonical targets of permitted Nix store/theme symlinks and any approved absolute local icons. The explicit disk directory should be dedicated to icon cache artifacts. Eviction removes only this cache's hashed entry names and leaves unrelated files intact.

## Retained divergences

- Explicit owners and directory arguments replace CUA's process-wide cache and write guard. Tests use temporary directories or memory-only owners.
- RAII retires canceled single-flight owners. A canceled GPUI background task cannot leave a closed channel that prevents later fetches.
- Decoder and disk read limits reject malformed or oversized PNGs before large allocations. Cache files and source reads reject nonregular files without blocking.
- Replacing an icon with a missing result removes stale positive variants. Changing PNG/SVG format removes the previous disk variant.
- Disk negative results retain their original age instead of restarting their TTL when read into memory.
- Atomic writes use unique temporary files, and pruning considers only owned hashed cache entry names.
- The Linux adapter preserves SVG, other GPUI formats, and high-density PNG paths. Its per-item batch results preserve partial useful output after a read failure.

`provenance.json` pins the original source hashes, local hashes, license, imported tests, and integration patches. The unchanged original library, tests, and Cargo manifest remain under `provenance/`. Unified source diffs are retained beside them. Cargo.lock pins the standalone dependency closure.

## Verification

Run from the repository root:

```sh
devenv shell -- bash crates/dojo-icon-cache/verify.sh
```

Tests exercise the retained cache behavior, cancellation/retry, format replacement, original negative TTL, disk corruption and pipe refusal, safe eviction, trusted local files, source changes, persisted PNG reuse, SVG/high-density compatibility, and per-item batch failures. A PNG fetch is also polled without a Tokio runtime, matching the GPUI background executor's integration requirement.

No root launcher files, user configuration, or installed desktop files are changed by this crate.
