# Perception artifact acquisition

The pinned worker source is CUA commit `88aff7e5424e9ed45b93ded85eb4111472ca5190`. Its source, tests and corpus are exact copies under `vendor/cua/driver/crates/cua-perception`, with copied-file ledger records. Its Python packaging scripts are archived upstream tooling, not production dependencies.

The public 0.2.1 release archive supplies model/runtime bytes. It was built from `6aa37d0751d094bfe8fdc2df331ed9eae934c832`; its executable is excluded from this integration. The archive identity and manifest hash are in `provenance/artifacts/cua-perception-release.lock.json`. Artifact hashes remain those from the baseline's existing artifact lock.

Reproduce acquisition outside tracked source:

```sh
mkdir -p /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/verified
curl --fail --location https://github.com/trycua/cua/releases/download/cua-perception-v0.2.1/cua-perception-0.2.1-x86_64-unknown-linux-gnu.tar.gz -o /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/linux-x64.tar.gz.part
sha256sum /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/linux-x64.tar.gz.part
```

Require the archive digest to equal the lock before extraction. Extract models, runtime, manifest and notices. Then independently verify both archive and extracted files:

```sh
tar -xzf /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/linux-x64.tar.gz.part -C /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/verified model-manifest.json models runtime notices metadata/provenance.redacted.json
node scripts/check-perception-artifacts.mjs /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/linux-x64.tar.gz.part /home/nathan/.cache/dojo/artifacts/cua-perception/0.2.1/verified
devenv shell -- cargo build --locked --manifest-path vendor/cua/driver/Cargo.toml -p cua-perception
devenv shell -- cargo test --locked --manifest-path vendor/cua/driver/Cargo.toml -p cua-perception
```

The worker tests passed 35 cases, with the separate real known-answer test ignored because its required image was absent. A standalone real CPU parse of the pinned `native-controls-light.png` succeeded, producing OCR and icon regions. The development check needed the exact devenv GCC 15.3 libstdc++ path in its loader environment. `/tmp/dojo-perception-real-inference.json` retains the synthetic-image result. This establishes real model execution, not contained service integration, real desktop accuracy or latency parity.

The mutable cache is acquisition state. Production loading must use a verified immutable installation and an explicit dependency closure. `nix-store --add` produced a root-owned, read-only copy at `/nix/store/6hqs2s1mv4ib73xmll7yj89hf6xqg9al-verified`; service integration is not yet configured to use it. Durable approvals remain a separate discussion.
