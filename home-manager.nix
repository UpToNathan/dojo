{ pkgs, lib, config, inputs }:
let
  homeDirectory = "/home/nathan";
  heliumVersion = "0.19.2.1";
  heliumAppImage = pkgs.fetchurl {
    url = "https://github.com/imputnet/helium-linux/releases/download/${heliumVersion}/helium-${heliumVersion}-x86_64.AppImage";
    hash = "sha256-oEVQo8fHC9rTrNOkQw7ajSr8C/cXOpxYpAP+q5UXCH8=";
  };
  helium = pkgs.appimageTools.wrapType2 {
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
  heliumContents = pkgs.appimageTools.extractType2 {
    pname = "helium";
    version = heliumVersion;
    src = heliumAppImage;
  };
  dojoAutostart = pkgs.writeShellScript "dojo-autostart" ''
    export LD_LIBRARY_PATH=${lib.escapeShellArg config.env.LD_LIBRARY_PATH}
    exec ${homeDirectory}/Dojo/target/debug/dojo --enable
  '';
in
{
      imports = [
        inputs.stasis.homeModules.default
        ./theme-home-manager.nix
      ];
      home.username = "nathan";
      home.homeDirectory = homeDirectory;
      home.stateVersion = "26.05";
      targets.genericLinux.enable = true;
      dojo.themeIntegrations.enable = true;
      nixpkgs.config.allowUnfreePredicate =
        pkg:
        builtins.elem (lib.getName pkg) [
          "1password-cli"
          "1password-gui"
          "1password"
        ];
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
        pkgs.zed-editor
        inputs.delta.packages.${pkgs.stdenv.hostPlatform.system}.delta
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
        pkgs.grim
        pkgs.slurp
        pkgs.wl-screenrec
        pkgs.procps
        pkgs.xdg-utils
        pkgs.speechd # Speech Dispatcher, including spd-say for browser speech synthesis
        (pkgs.writeShellScriptBin "toggle-vigiland" ''
          if ${pkgs.procps}/bin/pgrep -u "$UID" -x vigiland >/dev/null; then
            ${pkgs.procps}/bin/pkill -u "$UID" -x vigiland
          else
            ${inputs.vigiland.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/vigiland \
              >/dev/null 2>&1 </dev/null &
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
        inputs.opencode.packages.${pkgs.stdenv.hostPlatform.system}.default
        inputs.pi.packages.${pkgs.stdenv.hostPlatform.system}.default
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
        # btrbg: tiny Wayland wallpaper setter (not in nixpkgs)
        (pkgs.stdenv.mkDerivation {
          pname = "btrbg";
          version = "0-unstable-2026-10-06";
          src = builtins.fetchTarball {
            url = "https://codeberg.org/julmajustus/btrbg/archive/main.tar.gz";
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

      services.wluma = {
        enable = true;
        package = pkgs.wluma.overrideAttrs (old: {
          patches = (old.patches or []) ++ [ ./system-config/patches/wluma-no-reply.patch ];
        });
        systemd.target = "graphical-session.target";
        # No IIO/ALS hardware on venus; without this wluma panics trying to
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
        extraPathPackages = [ pkgs.niri ];
        extraConfig = ''
          default:
            enable_loginctl_integration true
            enable_dbus_inhibit true
            pause_on_lid_close true
            monitor_media false
            ignore_remote_media true
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

      xdg.configFile."xdg-desktop-portal/niri-portals.conf".text = ''
        [preferred]
        default=gtk
        org.freedesktop.impl.portal.Settings=darkman
        org.freedesktop.impl.portal.ScreenCast=gnome
        org.freedesktop.impl.portal.Screenshot=gnome
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
          shell = "/home/nathan/.nix-profile/bin/rush";
          hide_window_decorations = true;
          tab_bar_style = "hidden";
          confirm_os_window_close = 0;
          window_padding_width = "3 8";
          # launch --location=vsplit/hsplit only applies in Kitty's splits layout.
          enabled_layouts = "splits";
        };
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
        command = /home/nathan/.nix-profile/bin/rush
        font-family = Victor Mono
        font-size = 10
        font-feature = "ss01 1"
        gtk-wide-tabs = false
        confirm-close-surface = false
        window-show-tab-bar = never
        window-padding-x = 8
        window-padding-y = 3
        quick-terminal-position = center
        quick-terminal-size = 50%,50%
        initial-window = true
        gtk-single-instance = true
        quit-after-last-window-closed = false
        keybind = global:ctrl+f11=toggle_quick_terminal
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
      home.sessionVariables.NPM_CONFIG_PREFIX = "/home/nathan/.local/npm";
      home.sessionVariables.SSH_AUTH_SOCK = "/home/nathan/.1password/agent.sock";
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
      home.sessionPath = [ "/home/nathan/.local/share/mbx/bin" "/home/nathan/.nix-profile/bin" "/home/nathan/.local/npm/bin" "/home/nathan/.cargo/bin" "/home/nathan/.local/bin" ];
      home.file.".local/bin/codelldb".source = "${pkgs.vscode-extensions.vadimcn.vscode-lldb}/share/vscode/extensions/vadimcn.vscode-lldb/adapter/codelldb";
      home.file.".local/bin/dojo-niri-session".source = ./scripts/dojo-niri-session;
      xdg.configFile."systemd/user/wayland-wm@niri\\x2dsession.service.d/90-dojo-niri.conf".source = ./session-config/90-dojo-niri.conf;
      wayland.windowManager.niri = {
        enable = true;
	enableDefaultConfig = true;
	settings.binds."Mod+T" = {
	  _props.hotkey-overlay-title = "Open a Terminal: kitty";
	  spawn = [ "kitty" "--single-instance" ];
	};
	settings.binds."Mod+B" = {
	  _props.hotkey-overlay-title = "Open Helium";
	  spawn = [ "uwsm" "app" "-t" "service" "-d" "Helium" "--" "helium" ];
	};
	settings.binds."Mod+Shift+P" = {
	  _props.hotkey-overlay-title = "Toggle Dojo launcher";
	  spawn = [ "/home/nathan/Dojo/target/debug/dojo" "--launcher" ];
	};
	settings.binds."Ctrl+F11" = {
	  _props.hotkey-overlay-title = "Toggle quick-access terminal";
	  spawn = [ "kitten" "quick-access-terminal" ];
	};
	settings.binds."Mod+Shift+S" = {
	  _props.hotkey-overlay-title = "Select screenshot region to clipboard";
	  spawn = [ "sh" "-lc" ''
	    tmp="$(mktemp --suffix=.png)" || exit
	    trap 'rm -f "$tmp"' EXIT
	    geometry="$(slurp)" || exit
	    [ -n "$geometry" ] || exit
	    grim -g "$geometry" "$tmp" || exit
	    wl-copy --type image/png < "$tmp"
	  '' ];
	};
	settings.binds."Mod+Ctrl+Shift+S" = {
	  _props.hotkey-overlay-title = "Supersampled screenshot to clipboard";
	  spawn = [ "utsushot" "--copy" ];
	};
	settings.binds."Super+Alt+I" = {
	  _props.hotkey-overlay-title = "Toggle idle inhibition";
	  spawn = [ "toggle-vigiland" ];
	};
	settings.binds."Mod+Shift+D" = {
	  _props.hotkey-overlay-title = "Toggle light/dark mode";
	  spawn = [ "darkman" "toggle" ];
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
      settings.layout = {
        gaps = 16;
        focus-ring = {
          width = 3;
          # Fallback until Dojo has published a theme-specific focus ring.
          active-color = "#cba6f733"; # catppuccin mauve, 20% opacity
          inactive-color = "#45475a33";
        };
      };
      extraConfig = lib.mkAfter ''
        include optional=true "${homeDirectory}/.local/state/dojo/themes/niri.kdl"
      '';
      # Rounded corners on tiled windows
	settings."window-rule" = {
	  geometry-corner-radius = 12;
	  clip-to-geometry = true;
	  draw-border-with-background = false;
	};
	# Run btrbg with the wallpaper on login
  settings."spawn-at-startup" = [ "btrbg" "/home/nathan/Dojo/astro.jpg" ];
      };
}
