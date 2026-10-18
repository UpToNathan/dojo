# Copied CUA implementation

The baseline is CUA commit `afed1fcbba64acadcc154b8d5d377549ae62bc7e`. The earlier baseline `88aff7e5424e9ed45b93ded85eb4111472ca5190` still anchors the integrity ledger, the private Driver extraction and the Rust translations until the ledger migration described in [README.md](README.md) is reviewed.
`copy-plan.json` selects whole source trees. `copy-inventory.json` records each original Git object and maintained digest.

## Source and fork ownership

The import contains the full Rust Driver workspace, its Wayland helper resources, 23 MIT Cua workspace packages, three Fleet packages, image assets, contracts, fixtures, and notices. The existing private Driver extraction remains until its callers migrate.

The copier reads committed Git objects. Workspace membership changes and the filtered SDK lock are explicit metadata adaptations. Fork patches have pinned digests and declared output files. New adapter files have no fabricated upstream Git object.

The filtered SDK lock retains 596 upstream package identities. The filter removes 363 unused identities and changes no retained version or checksum. The raw Driver lock remains in `metadata/driver-upstream-Cargo.lock`. The Niri fork changes only shared libc, Serde, and serde_json versions to the existing Dojo pins, adds niri-ipc, and pins syn 3.0.5. All retained source identities and checksums remain unchanged.

`libs/cua/crates/cua-teleport/src/ux/targets.json` is FSL-1.1-MIT support data required by MIT telemetry. Its original license accompanies it. The Teleport implementation and other FSL server implementations are excluded. This source copy does not resolve distribution decisions.

## Pi integration

`tools/speech/cua-extension.mjs` connects Pi to the copied Rust CLI through the existing stdio MCP transport. The `cua_` prefix distinguishes resource tools from Dojo's desktop tools. Original argument schemas and multimodal results pass through unchanged.

`metadata/pi-resource-catalog.json` records 69 selected resource tools from the pinned CLI. Production connections must supply every selected schema unchanged. Unknown names do not enter Pi's registry. CUA's agent scheduler, provider calls, approval tools, and generic dispatch are excluded.

Pi remains the durable task owner, and pi-ai remains the provider transport. Unsafe replay declarations prevent automatic replay. They do not grant permission or implement an approval flow. This extension is opt-in through `config.pi.cuaTools`; it has not been deployed or enabled in installed configuration.

The CLI has a private `CUA_HOME` under Pi's state directory. Provider API keys are removed from its environment. Resource creation, cloud operations, and host installation have not been exercised by the integration smoke test.

## Niri fork

The first Niri backend unit is applied. It preserves original Driver handlers and adds bounded typed window, workspace, and output queries. Peer PID and process start bind each connection to one compositor generation. Rendered placements determine visible bounds; real workspace layout metadata supplies inactive-window bounds.

The Niri scrolling layout now exposes positions for inactive tiled windows. All 115 Niri layout tests pass, including that regression. All ten Driver Niri tests pass in the main checkout. Capture and native mutation routes still refuse before generic fallbacks until their coordinated backend is connected.

## Executable evidence

The original Driver contract suite passes 66 tests. Locked checks pass for the original Linux Driver SDK and copied sandbox SDK, CLI, and Spaces packages. SDK topology and creation progress suites pass 19 tests. CLI sandbox and MCP skills suites pass 18 tests with isolated loopback guest fixtures.

The real copied CLI negotiates MCP and serves 138 tools. The catalog check verifies all 69 selected resource tool definitions and an empty `list_spaces` result. A real Pi conversation calls that CLI and archives the result. The combined CUA and existing desktop Pi suites pass 29 tests.

These checks establish copied implementation and transport behavior. They do not establish full host, guest, hosted-product, inference, or evaluation parity.

## Reproduce checks

Use the exact pinned checkout as the source argument:

```sh
node vendor/cua/copy-upstream.mjs PINNED_CHECKOUT --verify
node --test vendor/cua/copy-upstream.test.mjs
node tools/speech/check-cua-catalog.mjs
DOJO_TEST_CUA_BINARY=/absolute/path/to/copied/cua node --test tools/speech/cua-extension.test.mjs
```

Build the copied CLI from Dojo's devenv before the real transport checks:

```sh
devenv shell -- cargo build --locked --manifest-path vendor/cua/upstream/libs/cua/Cargo.toml -p cua-cli
```

The live Niri fork uses `dojo-tests` for GUI acceptance. It must not activate the user's test workspace or fall back to uncontrolled primary input.
