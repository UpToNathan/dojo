{
  lib,
  stdenv,
  makeWrapper,
  wrapGAppsHook3,
  glib,
  gtk3,
  gtk4,
  unzip,
  at-spi2-atk,
  libdrm,
  libgbm,
  libxkbcommon,
  libxshmfence,
  libGL,
  vulkan-loader,
  alsa-lib,
  cairo,
  cups,
  dbus,
  expat,
  gdk-pixbuf,
  nss,
  nspr,
  libxrandr,
  libxfixes,
  libxext,
  libxdamage,
  libxcomposite,
  libx11,
  libxkbfile,
  libxcb,
  pango,
  systemd,
  pciutils,
  libnotify,
  pipewire,
  libsecret,
  libpulseaudio,
  speechd-minimal,
}:

version: distribution:
let
  pname = "electron";

  meta = {
    description = "Cross platform desktop application shell";
    homepage = "https://github.com/electron/electron";
    license = lib.licenses.mit;
    mainProgram = "electron";
    teams = [ lib.teams.electron ];
    platforms = [ "x86_64-linux" ];
    sourceProvenance = with lib.sourceTypes; [ binaryNativeCode ];
    knownVulnerabilities = lib.optional (lib.versionOlder version "42.0.0") "Electron version ${version} is EOL";
  };

  electronLibPath = lib.makeLibraryPath [
    alsa-lib
    at-spi2-atk
    cairo
    cups
    dbus
    expat
    gdk-pixbuf
    glib
    gtk3
    gtk4
    nss
    nspr
    libx11
    libxcb
    libxcomposite
    libxdamage
    libxext
    libxfixes
    libxrandr
    libxkbfile
    pango
    pciutils
    stdenv.cc.cc
    systemd
    libnotify
    pipewire
    libsecret
    libpulseaudio
    speechd-minimal
    libdrm
    libgbm
    libxkbcommon
    libxshmfence
    libGL
    vulkan-loader
  ];

  linux = finalAttrs: {
    buildInputs = [
      glib
      gtk3
      gtk4
    ];

    nativeBuildInputs = [
      unzip
      makeWrapper
      wrapGAppsHook3
    ];

    dontUnpack = true;
    dontBuild = true;

    installPhase = ''
      mkdir -p $out/libexec/electron
      unzip -d $out/libexec/electron $src
      chmod u-x $out/libexec/electron/*.so*
    '';

    # We don't want to wrap the contents of $out/libexec automatically
    dontWrapGApps = true;

    preFixup = ''
      makeWrapper "$out/libexec/electron/electron" $out/bin/electron \
        "''${gappsWrapperArgs[@]}" \
        --unset LD_LIBRARY_PATH --unset LD_PRELOAD
    '';

    postFixup = ''
      patchelf \
        --set-interpreter "$(cat $NIX_CC/nix-support/dynamic-linker)" \
        --set-rpath "${electronLibPath}:$out/libexec/electron" \
        $out/libexec/electron/electron \
        $out/libexec/electron/chrome_crashpad_handler

      # Electron 44 omits the unused ANGLE stub libraries.
      angleLibraries=( "$out/libexec/electron"/lib*GL* )
      if (( ''${#angleLibraries[@]} > 0 )) && [[ -e "''${angleLibraries[0]}" ]]; then
        patchelf \
          --set-rpath "${
            lib.makeLibraryPath [
              libGL
              pciutils
              vulkan-loader
            ]
          }" \
          "''${angleLibraries[@]}"

      fi

      # replace bundled vulkan-loader
      rm "$out/libexec/electron/libvulkan.so.1"
      ln -s -t "$out/libexec/electron" "${lib.getLib vulkan-loader}/lib/libvulkan.so.1"
    '';

    passthru.dist = finalAttrs.finalPackage + "/libexec/electron";

    __structuredAttrs = true;
    strictDeps = true;
  };

in
stdenv.mkDerivation (finalAttrs: (linux finalAttrs) // {
  inherit pname version meta;
  src = distribution;
})
