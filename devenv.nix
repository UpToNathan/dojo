{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

{
  # https://devenv.sh/basics/
  env.GREET = "devenv";
  env.DOJO_BROWSER_CONTROL_BROKER_EXE =
    "${(pkgs.callPackage ./nix/browser-control.nix { }).broker}/bin/dojo-browser-control-broker";
  env.DOJO_BROWSER_CONTROL_MCP_EXE =
    "${pkgs.callPackage ./nix/browser-control.nix { }}/bin/browser-control-mcp";
  env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
  env.BLA_VENDOR = "OpenBLAS";
  env.ALSA_CONFIG_PATH = "/usr/share/alsa/alsa.conf";
  env.ALSA_PLUGIN_DIR = "${pkgs.pipewire}/lib/alsa-lib";
  env.LD_LIBRARY_PATH = lib.makeLibraryPath [
    pkgs.libglvnd
    pkgs.vulkan-loader
    pkgs.mesa
    pkgs.wayland
    pkgs.openblas
    pkgs.alsa-lib
    pkgs.pipewire
  ];

  # https://devenv.sh/packages/
  packages = [
    pkgs.grim
    (pkgs.callPackage ./nix/rift.nix { })
    (pkgs.callPackage ./nix/browser-control.nix { })
    inputs.bend.packages.${pkgs.stdenv.hostPlatform.system}.default
    pkgs.pkg-config
    pkgs.protobuf
    pkgs.gnumake
    pkgs.readline.dev
    pkgs.cmake
    pkgs.shaderc
    pkgs.spirv-headers
    pkgs.vulkan-headers
    pkgs.openblas
    pkgs.alsa-lib
    pkgs.curl
    pkgs.fontconfig.dev
    pkgs.freetype.dev
    pkgs.wayland.dev
    pkgs.libxkbcommon.dev
    pkgs.libxi.dev
    pkgs.libxtst
    pkgs.openssl.dev
    pkgs.cairo.dev
    pkgs.dbus.dev
    pkgs.libdisplay-info_0_3
    pkgs.libinput.dev
    pkgs.seatd.dev
    pkgs.libgbm
    pkgs.pango.dev
    pkgs.systemd.dev
    pkgs.pipewire.dev
    pkgs.libglvnd
    pkgs.vulkan-loader
    pkgs.mesa
    pkgs.jq
  ];

  # https://devenv.sh/languages/
  # languages.rust.enable = true;

  # https://devenv.sh/processes/
  # processes.dev.exec = "${lib.getExe pkgs.watchexec} -n -- ls -la";

  # https://devenv.sh/services/
  # services.postgres.enable = true;

  # https://devenv.sh/scripts/
  scripts.hello.exec = ''
    echo hello from $GREET
  '';
  scripts.dojo-update.exec = ''
    exec ${pkgs.python3}/bin/python3 /home/nathan/Dojo/scripts/dojo-update "$@"
  '';
  scripts.dojo-voice.exec = ''
    exec /home/nathan/Dojo/tools/speech/dojo-voice "$@"
  '';
  scripts.install-rbonsai.exec = ''
    cargo install rbonsai --version '=0.1.5' --locked
  '';
  scripts.install-mbx.exec = ''
    cargo install mbx --version '=1.22.0' --locked
  '';
  scripts.install-nvim-lsps.exec = ''
    npm install --global --prefix /home/nathan/.local/npm \
      cssmodules-language-server@1.5.2 \
      gh-actions-language-server@0.0.3
  '';

  enterShell = ''
    hello # Run scripts directly
    # mbx's cargo shim shares compiled work across checkouts and Rift workspaces
    # and prunes its own storage. Without this the nix-profile cargo is used.
    if [ -x "$HOME/.local/share/mbx/bin/cargo" ]; then
      export PATH="$HOME/.local/share/mbx/bin:$PATH"
    fi
  '';

  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/tests/
  enterTest = ''
    echo "Running tests"
  '';

  # https://devenv.sh/git-hooks/
  # git-hooks.hooks.shellcheck.enable = true;

  # See full reference at https://devenv.sh/reference/options/
  languages.javascript = {
    enable = true;
    package = pkgs.nodejs;
  };

  machines.venus = {
    home-manager = import ./home-manager.nix {
      inherit pkgs lib config inputs;
    };
  };
}
