{ lib, stdenvNoCC, fetchFromGitHub, fetchurl, fetchPnpmDeps, pnpmConfigHook
, pnpm_11, nodejs, makeWrapper, rustPlatform, ffmpeg }:
let
  revision = "b4e4238742fa73daabfb664f1108113e9577849d";
  manifest = builtins.fromJSON (builtins.readFile ../vendor/browser-control/manifest.json);
  patch = ../vendor/browser-control/dojo.patch;
  buildId = "dojo-${revision}-${manifest.sourceIdentity}-${manifest.patchSha256}";
  src = fetchFromGitHub {
    owner = "anomalyco";
    repo = "browser-control";
    rev = revision;
    hash = "sha256-tqfKsvjIQQNVwx6OOhjCAdxbHQoE97W7XkgAIh82xic=";
  };
  pnpm = pnpm_11.overrideAttrs (_: {
    version = "11.20.0";
    src = fetchurl {
      url = "https://registry.npmjs.org/pnpm/-/pnpm-11.20.0.tgz";
      hash = "sha256-NOGYyx5DI3UX7O39MfmuJqbAo+U2bOWKLQX0sh+18Zo=";
    };
  });
  broker = rustPlatform.buildRustPackage {
    pname = "dojo-browser-control-broker";
    version = "0.1.0";
    src = lib.cleanSourceWith {
      src = ../crates/dojo-browser-control;
      filter = path: _: !(builtins.elem (baseNameOf path) [ "target" ".git" ]);
    };
    cargoLock.lockFile = ../crates/dojo-browser-control/Cargo.lock;
    doCheck = false;
  };
in
assert lib.assertMsg (manifest.revision == revision)
  "Browser Control manifest revision differs from the package pin";
assert lib.assertMsg (manifest.patchSha256 == builtins.hashFile "sha256" patch)
  "Browser Control patch changed; refresh its provenance manifest";
assert lib.assertMsg (lib.all (entry:
  entry.sha256 == builtins.hashFile "sha256"
    (builtins.toString ../crates/dojo-browser-control + "/${entry.path}")
) manifest.native) "Browser Control broker changed; refresh its provenance manifest";
assert lib.assertMsg (lib.all (entry:
  entry.sha256 == builtins.hashFile "sha256"
    (builtins.toString ../vendor/browser-control/upstream + "/${entry.path}")
) manifest.files) "Browser Control fork changed; refresh its provenance manifest";
stdenvNoCC.mkDerivation {
  pname = "dojo-browser-control";
  version = "0.8.4-dojo";
  inherit src;
  patches = [ patch ];
  pnpmDeps = fetchPnpmDeps {
    pname = "browser-control";
    inherit src pnpm;
    fetcherVersion = 4;
    hash = "sha256-a9cmsdEZO943nhjF2XhfvgKAtd01xRf9p0vDa7cv7eY=";
  };
  nativeBuildInputs = [ nodejs pnpm pnpmConfigHook makeWrapper ];
  DOJO_BROWSER_CONTROL_MODE = "1";
  DOJO_BROWSER_CONTROL_BUILD_ID = buildId;
  buildPhase = ''
    runHook preBuild
    pnpm build
    npm_config_offline=true pnpm prune --prod --ignore-scripts
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    package="$out/lib/browser-control"
    mkdir -p "$package" "$out/bin"
    cp -R dist extension skills node_modules package.json LICENSE README.md "$package/"
    for entry in cli mcp; do
      name=browser-control
      if [ "$entry" = mcp ]; then name=browser-control-mcp; fi
      makeWrapper ${nodejs}/bin/node "$out/bin/$name" \
        --add-flags "$package/dist/$entry.js" \
        --prefix PATH : ${lib.makeBinPath [ ffmpeg ]} \
        --set DOJO_BROWSER_CONTROL_MODE 1 \
        --set DOJO_BROWSER_CONTROL_BROKER_EXE ${broker}/bin/dojo-browser-control-broker
    done
    runHook postInstall
  '';
  passthru = { inherit revision pnpm broker buildId; extensionDirectory = "lib/browser-control/extension/dist"; };
  meta = {
    description = "Pinned Browser Control with Dojo native browser admission";
    homepage = "https://github.com/anomalyco/browser-control";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "browser-control";
  };
}
