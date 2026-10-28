{ pkgs, lib, config, inputs, homeDirectory, username, root }:
let
  # Hosts on the proprietary NVIDIA module record their driver here
  # (scripts/install-nvidia-gl.sh). Home Manager then builds the matching user-space
  # libraries into /run/opengl-driver; they must equal the loaded kernel module.
  # Without the file nothing changes, as on AMD and Intel hosts.
  nvidiaDriverFile = /etc/dojo/nvidia-driver.json;
  nvidiaDriver =
    if builtins.pathExists nvidiaDriverFile then
      builtins.fromJSON (builtins.readFile nvidiaDriverFile)
    else
      null;
  heliumVersion = "0.20.1.1";
  heliumAppImage = pkgs.fetchurl {
    url = "https://github.com/imputnet/helium-linux/releases/download/${heliumVersion}/helium-${heliumVersion}-x86_64.AppImage";
    hash = "sha256-1LeJsl0KmuEsdmceRE6Khe3Pgo4EGcZ40YxB+xrDAw4=";
  };
  heliumApp = pkgs.appimageTools.wrapType2 {
    pname = "helium";
    version = heliumVersion;
    src = heliumAppImage;
    # Chromium policies are outside the AppImage; expose Helium's shared,
    # system-managed policy directory read-only inside the FHS wrapper.
    extraBwrapArgs = [
      "--dir /etc/chromium"
      "--ro-bind-try /etc/chromium/policies /etc/chromium/policies"
    ];
  };
  helium = pkgs.writeShellScriptBin "helium" ''
    exec ${heliumApp}/bin/helium \
      --load-extension=${homeDirectory}/.local/share/dojo/browser-control/extension,${homeDirectory}/.local/share/dojo/tabs-extension "$@"
  '';
  heliumContents = pkgs.appimageTools.extractType2 {
    pname = "helium";
    version = heliumVersion;
    src = heliumAppImage;
  };
  dojoAutostart = pkgs.writeShellScript "dojo-autostart" ''
    export LD_LIBRARY_PATH=${lib.escapeShellArg config.env.LD_LIBRARY_PATH}
    exec ${root}/scripts/dojo-desktop --enable
  '';
  onepasswordAutostart = pkgs.writeShellScript "dojo-1password-autostart" ''
    onepassword=${pkgs._1password-gui}/bin/1password
    busctl=${pkgs.systemd}/bin/busctl
    sleep=${pkgs.coreutils}/bin/sleep
    attempt=0

    # Electron only registers the tray item when its StatusNotifier host
    # already exists. Wait for Dojo, then start 1Password silently.
    while [ "$attempt" -lt 300 ]; do
      if "$busctl" --user --quiet status org.freedesktop.StatusNotifierWatcher >/dev/null 2>&1; then
        exec "$onepassword" --silent
      fi
      attempt=$((attempt + 1))
      "$sleep" 1
    done

    # If Dojo never starts, show 1Password instead of leaving it hidden.
    exec "$onepassword"
  '';
  opencode = inputs.opencode.packages.${pkgs.stdenv.hostPlatform.system}.default.overrideAttrs (_: {
    # The current upstream CLI build imports script/native-arch.ts, which its
    # Nix node_modules source fileset leaves out.
    src = inputs.opencode.outPath;
  });
  pi = pkgs.callPackage ./nix/pi.nix { inherit inputs; };
  # Every lock trigger runs this: the Niri bind, stasis and the logind bridge.
  dojoLockNow = ./scripts/dojo-lock-now;
  dojoLockIdleSeconds = 300;
in
{
      imports = [
        inputs.stasis.homeModules.default
        inputs.rex.homeManagerModules.default
        ./rex-home-manager.nix
        ./paths-home-manager.nix
        ./theme-home-manager.nix
        ./t3-home-manager.nix
        ./browser-control-home-manager.nix
        ./rift-home-manager.nix
        ./voice-home-manager.nix
        ./native-accessibility-home-manager.nix
        ./qt6-annotation-home-manager.nix
        ./qt5-annotation-home-manager.nix
        ./yazi-home-manager.nix
        ./stasis-home-manager.nix
        ./update-home-manager.nix
        ./cache-home-manager.nix
        ./kitty-remote-control-home-manager.nix
        ./tabs-home-manager.nix
      ];
      home.username = username;
      dojo.root = root;
      home.homeDirectory = homeDirectory;
      home.stateVersion = "26.05";
      targets.genericLinux.enable = true;
      targets.genericLinux.gpu.nvidia = lib.mkIf (nvidiaDriver != null) {
        enable = true;
        inherit (nvidiaDriver) version sha256;
      };
      dojo.themeIntegrations.enable = true;
      programs.dojoNativeAccessibility.enable = true;
      programs.dojoQt6Annotation.enable = true;
      programs.dojoQt5Annotation.enable = true;
      nixpkgs.config.allowUnfreePredicate =
        pkg:
        builtins.elem (lib.getName pkg) (
          [
            "1password-cli"
            "1password-gui"
            "1password"
          ]
          ++ lib.optional (nvidiaDriver != null) "nvidia-x11"
        );
      nixpkgs.config.nvidia.acceptLicense = nvidiaDriver != null;
      # _1password-cli is installed at /usr/local/bin/op with the required
      # onepassword-cli setgid group; a Nix-store binary cannot carry that bit.
      home.packages = with pkgs; [
        _1password-gui
        legcord
        helium
        (pkgs.writeShellScriptBin "dojo-chromium-theme-policy" ''
          exec /usr/bin/sudo -n /usr/local/bin/dojo-chromium-theme-policy "$@"
        '')
        brightnessctl
        wlsunset
        pkgs.git
        pkgs.openssh
        pkgs.neovim
        pkgs.cargo
        pkgs.rustc
        pkgs.tree-sitter
        pkgs.stdenv.cc
        inputs.bend.packages.${pkgs.stdenv.hostPlatform.system}.default
        pkgs.nodejs
        pkgs.lua-language-server
        pkgs.typescript
        pkgs.vscode-langservers-extracted
        pkgs.css-variables-language-server
        pkgs.tailwindcss-language-server
        pkgs.bash-language-server
        pkgs.fish-lsp
        pkgs.just-lsp
        pkgs.gopls
        pkgs.crystalline
        pkgs.yaml-language-server
        pkgs.dockerfile-language-server
        pkgs.docker-compose-language-service
        pkgs.systemd-lsp
        pkgs.deno
        pkgs.oxlint
        pkgs.sqruff
        pkgs.taplo
        pkgs.go
        pkgs.delve
        pkgs.rustfmt
        pkgs.hunk
        pkgs.vscode-extensions.vadimcn.vscode-lldb
        pkgs.vscode-js-debug
        pkgs.wl-clipboard
        pkgs.wl-screenrec
        pkgs.procps
        pkgs.xdg-utils
        pkgs.speechd # Speech Dispatcher, including spd-say for browser speech synthesis
        (pkgs.writeShellScriptBin "toggle-vigiland" ''
          if ${pkgs.procps}/bin/pgrep -u "$UID" -x vigiland >/dev/null; then
            ${pkgs.procps}/bin/pkill -u "$UID" -x vigiland
          else
            # Niri ignores vigiland's idle inhibitor (its surface is never shown), so hold
            # stasis's timers directly. A stasis restart (a deployment, a crash) forgets a manual
            # pause, so keep asserting it while vigiland runs and release it when vigiland exits,
            # however it ends.
            (
              ${pkgs.systemd}/bin/systemd-inhibit \
                --what=handle-lid-switch \
                --mode=block \
                --who=Vigiland \
                --why='Vigiland is keeping the machine awake' \
                ${inputs.vigiland.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/vigiland &
              holder=$!
              while kill -0 "$holder" 2>/dev/null; do
                ${inputs.stasis.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/stasis pause
                sleep 20
              done
              ${inputs.stasis.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/stasis resume
            ) >/dev/null 2>&1 </dev/null &
          fi
        '')
        (pkgs.writeShellScriptBin "toggle-wlsunset" ''
          if ${pkgs.systemd}/bin/systemctl --user is-active --quiet wlsunset.service; then
            exec ${pkgs.systemd}/bin/systemctl --user stop wlsunset.service
          else
            exec ${pkgs.systemd}/bin/systemctl --user start wlsunset.service
          fi
        '')
        inputs.clipvault.packages.${pkgs.stdenv.hostPlatform.system}.default
        inputs.utsushot.packages.${pkgs.stdenv.hostPlatform.system}.default
        inputs.vigiland.packages.${pkgs.stdenv.hostPlatform.system}.default
        opencode
        pi
        pkgs.xdg-desktop-portal
        pkgs.xdg-desktop-portal-gtk
        pkgs.xdg-desktop-portal-gnome
        # Rush shell (from its own flake; includes Zig build)
        inputs.rush.packages.${pkgs.stdenv.hostPlatform.system}.default
        # Victor Mono font (downloaded from upstream)
        (pkgs.stdenv.mkDerivation {
          pname = "victor-mono";
          version = "all";
          src = pkgs.fetchzip {
            url = "https://rubjo.github.io/victor-mono/VictorMonoAll.zip";
            sha256 = "sha256-sTehXK6qqGwgXyUgTTQPTu/fmnbPPrY5w+qLDyYvvtA=";
            stripRoot = false;
          };
          dontBuild = true;
          installPhase = ''
            mkdir -p $out/share/fonts
            find . -name '*.ttf' -exec cp {} $out/share/fonts \;
          '';
        })
        # Victor Mono with ss01 (single-storey a) frozen in as the default, as "Victor Mono SS01".
        # Chromium and Electron apps such as T3 Code cannot be handed OpenType features from
        # outside, so they select this family in their font setting instead. The files carry an SS01- prefix
        # because the plain victor-mono package installs the same filenames into the same profile.
        (pkgs.runCommand "victor-mono-ss01" {
          nativeBuildInputs = [ pkgs.python3Packages.opentype-feature-freezer ];
          src = pkgs.fetchzip {
            url = "https://rubjo.github.io/victor-mono/VictorMonoAll.zip";
            sha256 = "sha256-sTehXK6qqGwgXyUgTTQPTu/fmnbPPrY5w+qLDyYvvtA=";
            stripRoot = false;
          };
        } ''
          mkdir -p $out/share/fonts
          find $src -name '*.ttf' | while read -r font; do
            pyftfeatfreeze -f ss01 -S -U SS01 "$font" "$out/share/fonts/SS01-$(basename "$font")"
          done
        '')
        # Colour emoji: GPUI falls back to a system "NotoColorEmoji" face for them, and without
        # one every emoji in the shell's speech bubbles draws as an empty box.
        pkgs.noto-fonts-color-emoji
        # btrbg: tiny Wayland wallpaper setter (not in nixpkgs)
        (pkgs.stdenv.mkDerivation {
          pname = "btrbg";
          version = "0-unstable-2026-10-06";
          src = builtins.fetchTarball {
            url = "https://codeberg.org/julmajustus/btrbg/archive/2d36d378f7779fb91e29440bca1f701d9b6bfcf8.tar.gz";
            sha256 = "sha256:1dmz3p2jbncll8x071iarlnz3x0zbs9prwaf2xll0jk2r8sgdss8";
          };
          nativeBuildInputs = [ pkg-config wayland-scanner wayland-protocols ];
          buildInputs = [ wayland libxkbcommon ];
          makeFlags = [ "release" ];
          installPhase = ''
            mkdir -p $out/bin
            cp btrbg $out/bin/btrbg
          '';
        })
        # Zen with the 1Password extension pre-installed via policy
        (pkgs.wrapFirefox
          inputs.zen-browser.packages.${pkgs.stdenv.hostPlatform.system}.zen-browser-unwrapped
          {
            extraPolicies = {
              DisableTelemetry = true;
              ExtensionSettings = {
                "{d634138d-c276-4fc8-924b-40a0ea21d284}" = {
                  install_url = "https://addons.mozilla.org/en-US/firefox/downloads/latest/1password-x-password-manager/latest.xpi";
                  installation_mode = "normal_installed";
                };
              };
            };
        })
      ];
      xdg.desktopEntries.helium = {
        name = "Helium";
        genericName = "Web Browser";
        exec = "helium %U";
        icon = "${heliumContents}/helium.png";
        terminal = false;
        startupNotify = true;
        categories = [ "Network" "WebBrowser" ];
        mimeType = [
          "application/pdf"
          "application/rdf+xml"
          "application/rss+xml"
          "application/xhtml+xml"
          "application/xhtml_xml"
          "application/xml"
          "image/gif"
          "image/jpeg"
          "image/png"
          "image/webp"
          "text/html"
          "text/xml"
          "x-scheme-handler/http"
          "x-scheme-handler/https"
        ];
      };
      fonts.fontconfig.enable = true;
      xdg.configFile."nvim".source = ./nvim-config;
      xdg.configFile."rush/config.rush".source = ./rush-config/config.rush;
      # uwsm app -T (the launcher's terminal-app path) picks its terminal through xdg-terminal-exec and
      # ignores $TERMINAL; without this list it falls back to whichever terminal sorts first.
      xdg.configFile."xdg-terminals.list".text = "kitty.desktop\n";

      services.wluma = {
        # Paused pending power measurements of screen-content capture.
        enable = false;
        package = pkgs.wluma.overrideAttrs (old: {
          patches = (old.patches or []) ++ [ ./system-config/patches/wluma-no-reply.patch ];
        });
        systemd.target = "graphical-session.target";
        # No IIO/ALS hardware on this machine; without this wluma panics trying to
        # enumerate IIO devices. It still learns from screen contents + your
        # manual brightness adjustments. Also no intel/dell paths exist here,
        # so auto-discovery is overridden with the actual devices.
        settings = {
          als.none = { };
          output.backlight = [
            { name = "eDP-1"; path = "/sys/class/backlight/amdgpu_bl1"; }
          ];
          keyboard = [
            { name = "keyboard"; path = "/sys/class/leds/tpacpi::kbd_backlight"; }
          ];
        };
      };

      services.darkman = {
        enable = true;
        settings = {
          portal = true;
          # Automatic sunrise/sunset switching needs an explicit location or
          # a system GeoClue service; don't guess the user's location.
          usegeoclue = false;
        };
      };

      services.stasis = {
        enable = true;
        extraPathPackages = [ pkgs.niri pkgs.brightnessctl ];
        extraConfig = ''
          default:
            enable_loginctl_integration true
            enable_dbus_inhibit true
            pause_on_lid_close true
            monitor_media false
            ignore_remote_media true

            # Stasis runs the steps directly under `default:` on desktops only. On a laptop it
            # ignores them and runs `ac:` or `battery:`, so the idle lock goes in all three.
            lock_screen:
              timeout ${toString dojoLockIdleSeconds}
              command "${dojoLockNow}"
            end

            # Stasis timeouts are relative to the previous enabled step, so each plan below lists
            # the delay since the step before it. Totals on battery: dim at 2 minutes, lock at 5,
            # screen off at 5.5, suspend-then-hibernate at 15 (Arch Wiki, Power management:
            # lower brightness, then blank the screen, then sleep). On AC the screen only locks
            # and blanks; nothing suspends.
            ac:
              lock_screen:
                timeout ${toString dojoLockIdleSeconds}
                command "${dojoLockNow}"
              end

              dpms:
                timeout 300
                command "niri msg action power-off-monitors"
                resume_command "niri msg action power-on-monitors"
              end
            end

            battery:
              brightness:
                timeout 120
                command "brightnessctl -s set 25%"
                resume_command "brightnessctl -r"
              end

              lock_screen:
                timeout ${toString (dojoLockIdleSeconds - 120)}
                command "${dojoLockNow}"
              end

              dpms:
                timeout 30
                command "niri msg action power-off-monitors"
                resume_command "niri msg action power-on-monitors"
              end

              suspend:
                timeout 570
                command "${homeDirectory}/.local/bin/dojo-desktop --leave suspend-then-hibernate"
              end
            end
          end
        '';
      };
      # 1Password's Linux CLI integration requires a PolKit authentication
      # agent in the graphical session; polkitd alone is not sufficient.
      services.polkit-gnome.enable = true;

      services.gnome-keyring = {
        enable = true;
        components = [ "secrets" ];
      };
      xdg.dataFile."dbus-1/services/org.gnome.keyring.SystemPrompter.service".source =
        "${pkgs.gcr_3}/share/dbus-1/services/org.gnome.keyring.SystemPrompter.service";
      xdg.dataFile."dbus-1/services/org.gnome.keyring.PrivatePrompter.service".source =
        "${pkgs.gcr_3}/share/dbus-1/services/org.gnome.keyring.PrivatePrompter.service";

      # Restart only after a crash (signal or non-zero exit). A clean Quit or
      # a session stop is left alone, unlike Restart=always.
      xdg.configFile."systemd/user/app-1password@autostart.service.d/10-dojo-tray.conf".text = ''
        [Unit]
        StartLimitIntervalSec=10min
        StartLimitBurst=5

        [Service]
        ExecStart=
        ExecStart=${onepasswordAutostart}
        Restart=on-failure
        RestartSec=3s
      '';
      xdg.configFile."autostart/dojo.desktop".text = ''
        [Desktop Entry]
        Name=Dojo
        Comment=Dojo desktop shell and system tray host
        Exec=${dojoAutostart}
        Terminal=false
        Type=Application
        StartupNotify=false
        X-GNOME-Autostart-enabled=true
      '';
      xdg.configFile."autostart/1password.desktop".text = ''
        [Desktop Entry]
        Name=1Password
        Comment=Password manager and secure wallet
        Exec=${onepasswordAutostart}
        Icon=1password
        Terminal=false
        Type=Application
        StartupNotify=false
        X-GNOME-Autostart-enabled=true
      '';
      # The tailscale package comes from the system (see system-config); TryExec skips the
      # entry on a host without it.
      xdg.configFile."autostart/tailscale-systray.desktop".text = ''
        [Desktop Entry]
        Name=Tailscale System Tray
        Comment=Tailscale system tray applet for managing Tailscale
        Exec=/usr/bin/tailscale systray
        TryExec=/usr/bin/tailscale
        Icon=tailscale
        Terminal=false
        Type=Application
        NoDisplay=true
        StartupNotify=false
        X-GNOME-Autostart-enabled=true
      '';
      systemd.user.packages = with pkgs; [
        xdg-desktop-portal
        xdg-desktop-portal-gtk
        xdg-desktop-portal-gnome
      ];

      systemd.user.sockets.speech-dispatcher = {
        Unit.Description = "Speech Dispatcher socket";
        Socket.ListenStream = "%t/speech-dispatcher/speechd.sock";
        Install.WantedBy = [ "sockets.target" ];
      };
      systemd.user.services.speech-dispatcher = {
        Unit = {
          Description = "Speech Dispatcher daemon";
          Requires = [ "speech-dispatcher.socket" ];
        };
        Service = {
          Type = "simple";
          ExecStart = "${pkgs.speechd}/bin/speech-dispatcher -s -t 0";
        };
      };
      # Started on demand by dojo-lock-now, never at login. A crashed locker is
      # restarted while the compositor keeps the session locked; a second locker
      # exits 0. Override the Vulkan pin with a drop-in that sets VK_DRIVER_FILES.
      systemd.user.services.dojo-lock = {
        Unit = {
          Description = "Dojo lock screen";
          After = [ "graphical-session.target" ];
          PartOf = [ "graphical-session.target" ];
          ConditionEnvironment = "WAYLAND_DISPLAY";
          StartLimitIntervalSec = 0;
          X-SwitchMethod = "keep-old";
        };
        Service = {
          Type = "simple";
          ExecStart = "${homeDirectory}/.local/bin/dojo-lock";
          Environment = [
            "VK_DRIVER_FILES=/run/opengl-driver/share/vulkan/icd.d/${
              if nvidiaDriver != null then "nvidia_icd.json" else "radeon_icd.x86_64.json"
            }"
          ];
          Restart = "on-failure";
          RestartSec = "250ms";
          KillMode = "control-group";
          TimeoutStopSec = "5s";
        };
      };
      # stasis cannot hold a sleep delay inhibitor or react to `loginctl lock-session`. The bridge
      # also holds a shutdown delay inhibitor and asks the shell for its farewell on any poweroff or
      # restart, `systemctl poweroff` included, then releases it.
      systemd.user.services.dojo-lock-session-bridge = {
        Unit = {
          Description = "Lock Dojo on loginctl lock-session and show its farewell before suspend and shutdown";
          After = [ "graphical-session.target" ];
          PartOf = [ "graphical-session.target" ];
          ConditionEnvironment = "WAYLAND_DISPLAY";
        };
        Service = {
          ExecStart = "${pkgs.bash}/bin/bash ${./scripts/dojo-lock-session-bridge}";
          Environment = [
            "PATH=${lib.makeBinPath [ pkgs.coreutils pkgs.gnused pkgs.systemd pkgs.glib pkgs.util-linux ]}"
            "DOJO_LOCK_NOW=${dojoLockNow}"
          ];
          Restart = "on-failure";
          RestartSec = "5s";
        };
        Install.WantedBy = [ "graphical-session.target" ];
      };

      systemd.user.services.wlsunset = {
        Unit = {
          Description = "Warm screen color temperature adjustment";
          After = [ "graphical-session.target" ];
          PartOf = [ "graphical-session.target" ];
          ConditionEnvironment = "WAYLAND_DISPLAY";
        };
        Service = {
          ExecStart = "${pkgs.wlsunset}/bin/wlsunset -t 1000 -T 6500";
          Restart = "on-failure";
        };
      };

      xdg.configFile."speech-dispatcher/speechd.conf".text = ''
        AddModule "espeak-ng" "${pkgs.speechd}/libexec/speech-dispatcher-modules/sd_espeak-ng" "espeak-ng.conf"
        DefaultModule espeak-ng
      '';
      xdg.configFile."speech-dispatcher/espeak-ng.conf".source = "${pkgs.speechd}/etc/speech-dispatcher/modules/espeak-ng.conf";

      # History stores past entries, but does not keep the active Wayland
      # selection alive when its source application exits.
      systemd.user.services.wl-clip-persist = {
        Unit = {
          Description = "Keep the Wayland clipboard after its source exits";
          After = [ "graphical-session.target" ];
          PartOf = [ "graphical-session.target" ];
          ConditionEnvironment = "WAYLAND_DISPLAY";
        };
        Service = {
          ExecStart = "${pkgs.wl-clip-persist}/bin/wl-clip-persist --clipboard regular";
          Restart = "on-failure";
        };
        Install.WantedBy = [ "graphical-session.target" ];
      };

      systemd.user.services.clipvault-history = {
        Unit = {
          Description = "Store Wayland clipboard entries in Clipvault history";
          After = [ "graphical-session.target" ];
          PartOf = [ "graphical-session.target" ];
          ConditionEnvironment = "WAYLAND_DISPLAY";
        };
        Service = {
          ExecStart = "${pkgs.wl-clipboard}/bin/wl-paste --watch ${inputs.clipvault.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/clipvault store";
          Restart = "on-failure";
        };
        Install.WantedBy = [ "graphical-session.target" ];
      };

      programs.yazi = {
        enable = true;
        package = pkgs.yazi.override { _7zz = pkgs._7zz-rar; };
        settings.opener.edit = [
          {
            run = "nvim %s";
            block = true;
            for = "unix";
          }
        ];
        extraPackages = with pkgs; [
          file
          ffmpeg
          _7zz-rar
          jq
          poppler
          fd
          ripgrep
          fzf
          zoxide
          resvg
          imagemagick
          wl-clipboard
        ];
      };

      programs.ghostty = {
        enable = true;
        systemd.enable = true; # keep a running instance via systemd user service on Linux
        settings = { };
      };
      programs.kitty = {
        enable = true;
        settings = {
          font_family = "family=\"Victor Mono\" features=\"+ss01\"";
          font_size = 10;
          background_opacity = 1.0;
          cursor_shape = "beam";
          mouse_hide_wait = -1;
          url_style = "straight";
          detect_urls = true;
          focus_follows_mouse = true;
          strip_trailing_spaces = "always";
          shell = "${homeDirectory}/.nix-profile/bin/rush";
          hide_window_decorations = true;
          tab_bar_style = "hidden";
          confirm_os_window_close = 0;
          window_padding_width = "3 8";
          # launch --location=vsplit/hsplit only applies in Kitty's splits layout.
          enabled_layouts = "splits";
          # Remote control only with a password, from the socket and the terminal alike, so nothing
          # running inside kitty (or reaching it over SSH) can drive it. Dojo's own password, written
          # by the activation below and limited to `ls`, lets it read a window's directory and
          # program to send the window to another machine (docs/mesh.md). The socket is private.
          allow_remote_control = "password";
          listen_on = "unix:\${XDG_RUNTIME_DIR}/kitty-{kitty_pid}.sock";
        };
        extraConfig = ''
          include dojo-remote-control.conf
        '';
        shellIntegration.mode = "enabled";
        keybindings = {
          "f9" = "load_config_file";
          "ctrl+g>shift+backslash" = "launch --location=vsplit --cwd=current";
          "ctrl+g>minus" = "launch --location=hsplit --cwd=current";
          "ctrl+g>ctrl+t" = "new_tab";
          "ctrl+g>ctrl+left" = "previous_tab";
          "ctrl+g>ctrl+right" = "next_tab";
          "ctrl+g>ctrl+h" = "neighboring_window left";
          "ctrl+g>ctrl+l" = "neighboring_window right";
          "ctrl+g>ctrl+k" = "neighboring_window top";
          "ctrl+g>ctrl+j" = "neighboring_window down";
          "ctrl+g>ctrl+shift+left" = "resize_window narrower 10";
          "ctrl+g>ctrl+shift+right" = "resize_window wider 10";
          "ctrl+g>ctrl+shift+up" = "resize_window shorter 10";
          "ctrl+g>ctrl+shift+down" = "resize_window taller 10";
          "ctrl+g>q" = "close_window";
          "page_up" = "scroll_to_prompt -1";
          "page_down" = "scroll_to_prompt 1";
        };
        quickAccessTerminalConfig = {
          edge = "center-sized";
          lines = 30;
          columns = 100;
          background_opacity = 1.0;
          hide_on_focus_loss = false;
          start_as_hidden = false;
        };
      };
      # Ghostty config (custom text since it uses duplicate keys that
      # home-manager's settings attrset can't express)
      xdg.configFile."ghostty/config".text = ''
        window-decoration = false
        background-opacity = 1.0
        cursor-style = bar
        mouse-hide-while-typing = true
        link-url = true
        focus-follows-mouse = true
        clipboard-trim-trailing-spaces = true
        shell-integration-features = true
        command = ${homeDirectory}/.nix-profile/bin/rush
        font-family = Victor Mono
        font-size = 10
        font-feature = "ss01 1"
        gtk-wide-tabs = false
        confirm-close-surface = false
        window-show-tab-bar = never
        window-padding-x = 8
        window-padding-y = 3
        initial-window = true
        gtk-single-instance = true
        quit-after-last-window-closed = false
        keybind = all:f9=reload_config
        keybind = ctrl+g>shift+backslash=new_split:right
        keybind = ctrl+g>-=new_split:down
        keybind = all:f12=toggle_tab_overview
        keybind = ctrl+g>ctrl+t=new_tab
        keybind = ctrl+g>ctrl+left=previous_tab
        keybind = ctrl+g>ctrl+right=next_tab
        keybind = ctrl+g>ctrl+h=goto_split:left
        keybind = ctrl+g>ctrl+l=goto_split:right
        keybind = ctrl+g>ctrl+k=goto_split:up
        keybind = ctrl+g>ctrl+j=goto_split:down
        keybind = ctrl+g>f9=equalize_splits
        keybind = ctrl+g>ctrl+shift+left=resize_split:left,10
        keybind = ctrl+g>ctrl+shift+right=resize_split:right,10
        keybind = ctrl+g>ctrl+shift+up=resize_split:up,10
        keybind = ctrl+g>ctrl+shift+down=resize_split:down,10
        keybind = ctrl+g>q=close_surface
        keybind = all:page_down=jump_to_prompt:1
        keybind = all:page_up=jump_to_prompt:-1
      '';
      home.sessionVariables.NPM_CONFIG_PREFIX = "${homeDirectory}/.local/npm";
      home.sessionVariables.SSH_AUTH_SOCK = "${homeDirectory}/.1password/agent.sock";
      home.sessionVariables.BROWSER = "helium";
      home.sessionVariables.EDITOR = "nvim";
      home.sessionVariables.VISUAL = "nvim";
      home.sessionVariables.TERMINAL = "kitty";
      systemd.user.sessionVariables = {
        BROWSER = "helium";
        EDITOR = "nvim";
        VISUAL = "nvim";
        TERMINAL = "kitty";
      };
      home.sessionPath = map (path: "${homeDirectory}/${path}") [ ".local/share/mbx/bin" ".nix-profile/bin" ".local/npm/bin" ".cargo/bin" ".local/bin" ];
      home.file.".local/bin/codelldb".source = "${pkgs.vscode-extensions.vadimcn.vscode-lldb}/share/vscode/extensions/vadimcn.vscode-lldb/adapter/codelldb";
      home.file.".local/bin/dojo-niri-session".source = ./scripts/dojo-niri-session;
      home.file.".local/bin/dojo-voice".source = pkgs.writeShellScript "dojo-voice" ''
        exec ${root}/tools/speech/dojo-voice "$@"
      '';
      xdg.configFile."systemd/user/wayland-wm@niri\\x2dsession.service.d/90-dojo-niri.conf".source = ./session-config/90-dojo-niri.conf;
      wayland.windowManager.niri = {
        enable = true;
        checkConfig = false;
	settings.input.focus-follows-mouse = {};
	# Pointing-device sections replace the default config's rather than merge, so tap and
	# natural-scroll are repeated. dwt ignores the touchpad while typing, so a bumped palm
	# neither moves focus nor clicks.
	settings.input.touchpad = { tap = {}; natural-scroll = {}; dwt = {}; };
	# Niri hardcodes the power key to suspend, ahead of any bind, unless this is set.
	settings.input.disable-power-key-handling = {};
	# Niri's hotkey overlay is drawn over everything at every session start. It would be the first
	# thing seen after the login screen's fade, ahead of the shell's own first frame.
	settings.hotkey-overlay.skip-at-startup = {};
	enableDefaultConfig = true;
	settings.binds."Mod+T" = {
	  _props.hotkey-overlay-title = "Open a Terminal: kitty";
	  spawn = [ "kitty" "--single-instance" ];
	};
	settings.binds."Mod+B" = {
	  _props.hotkey-overlay-title = "Open Helium";
	  spawn = [ "uwsm" "app" "-t" "service" "-d" "Helium" "--" "helium" ];
	};
	settings.binds."Mod+Space" = {
	  _props.hotkey-overlay-title = "Toggle Dojo launcher";
	  spawn = [ "${root}/scripts/dojo-desktop" "--launcher" ];
	};
	settings.binds."Mod+Shift+P" = {
	  _props.hotkey-overlay-title = "Hold to talk to Dojo";
          _props.repeat = false;
          _props.spawn-on-release = "${root}/tools/speech/dojo-voice finish";
	  spawn = [ "${root}/tools/speech/dojo-voice" "begin" ];
	};
	settings.binds."Mod+Shift+F23" = {
	  _props.hotkey-overlay-title = "Hold to talk to Dojo";
          _props.repeat = false;
          _props.spawn-on-release = "${root}/tools/speech/dojo-voice finish";
	  spawn = [ "${root}/tools/speech/dojo-voice" "begin" ];
	};
	settings.binds."Mod+Shift+Escape" = {
	  _props.hotkey-overlay-title = "Cancel Dojo recording";
	  spawn = [ "${root}/tools/speech/dojo-voice" "cancel" ];
	};
	settings.binds."Ctrl+F11" = {
	  _props.hotkey-overlay-title = "Toggle quick-access terminal";
	  spawn = [ "${homeDirectory}/.local/bin/dojo-desktop" "--terminal" ];
	};
	settings.binds."Mod+F11" = {
	  _props.hotkey-overlay-title = "Send a window to another machine";
	  spawn = [ "${homeDirectory}/.local/bin/dojo-desktop" "--send-window" ];
	};
	# Niri's default Print binds copy to the clipboard and also write a PNG under ~/Pictures/Screenshots.
	# The interactive screenshot has no per-bind switch for the file, so none is written at all.
	settings.screenshot-path = null;
	settings.binds."Mod+Ctrl+Shift+S" = {
	  _props.hotkey-overlay-title = "Supersampled screenshot to clipboard";
	  spawn = [ "utsushot" "--copy" ];
	};
	settings.binds."Super+Alt+I" = {
	  _props.hotkey-overlay-title = "Toggle idle inhibition";
	  spawn = [ "toggle-vigiland" ];
	};
	# Hands the desktop back to the agent after the person has used the keyboard or mouse. The
	# stock `niri` on PATH has no such command; the compositor's own binary does.
	settings.binds."Super+Alt+Y" = {
	  _props.hotkey-overlay-title = "Yield the desktop and browser to the agent";
	  spawn = [ "${homeDirectory}/.local/share/dojo/niri/current/niri" "msg" "yield-human-input" ];
	};
	settings.binds."Super+Alt+L" = {
	  _props.hotkey-overlay-title = "Lock the screen";
	  spawn = [ "${dojoLockNow}" ];
	};
	settings.binds."Mod+Shift+D" = {
	  _props.hotkey-overlay-title = "Toggle light/dark mode";
	  spawn = [ "darkman" "toggle" ];
	};
	# logind ignores a short press (system-config/systemd/logind.conf.d/90-dojo-power-key.conf);
	# a long press still powers off. The lid stays with logind: the closing cannot be seen
	# once the lid is shut, and the bridge covers the sleep that follows.
	settings.binds."XF86PowerOff" = {
	  spawn = [ "${homeDirectory}/.local/bin/dojo-desktop" "--leave" "shutdown" ];
	};
	settings.binds."XF86MonBrightnessUp" = {
	  spawn = [ "brightnessctl" "set" "+5%" ];
	};
	settings.binds."XF86MonBrightnessDown" = {
	  spawn = [ "brightnessctl" "set" "5%-" ];
	};
	settings.binds."XF86KbdBrightnessUp" = {
	  spawn = [ "brightnessctl" "--device" "tpacpi::kbd_backlight" "set" "+1" ];
	};
	settings.binds."XF86KbdBrightnessDown" = {
	  spawn = [ "brightnessctl" "--device" "tpacpi::kbd_backlight" "set" "1-" ];
	};
      # Niri paints the output backdrop in the gaps a workspace swipe reveals; it defaults to grey.
      # Fallback until the theme export publishes it in themes/niri.kdl, like the layout colour.
      settings.overview.backdrop-color = "#1e1e2e";
      settings.layout = {
        gaps = 16;
        # Dojo frame colour (ThemeColors default background) until the theme
        # export publishes the palette background in themes/niri.kdl.
        background-color = "#1e1e2e";
        focus-ring = {
          width = 3;
          # Fallback until Dojo has published a theme-specific focus ring.
          active-color = "#cba6f733"; # catppuccin mauve, 20% opacity
          inactive-color = "#45475a33";
        };
      };
      extraConfig = lib.mkAfter ''
        include optional=true "${homeDirectory}/.local/state/dojo/displays/niri.kdl"
        include optional=true "${homeDirectory}/.local/state/dojo/themes/niri.kdl"
      '';
      # Rounded corners on tiled windows
	settings."window-rule" = {
	  geometry-corner-radius = 12;
	  clip-to-geometry = true;
	  draw-border-with-background = false;
	};
	# Dojo surfaces use backdrop blur over the windows underneath.
	settings."layer-rule" = {
	  match._props.namespace = "^dojo-frame-[0-9]+$";
	  background-effect.xray = false;
	};
	# Dojo restores its selected wallpaper after login.
      };
}
