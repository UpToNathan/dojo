#!/usr/bin/env bash
set -euo pipefail
artifact=$(readlink -f "${1:?Pass the scoped Yazi module artifact}")
config_root=${XDG_CONFIG_HOME:-$HOME/.config}
data_root=${XDG_DATA_HOME:-$HOME/.local/share}
link_file() {
    local source=$1 target=$2
    [[ -f $source ]] || { printf 'Missing artifact: %s\n' "$source" >&2; exit 1; }
    mkdir -p "$(dirname "$target")"
    if [[ -L $target && $(readlink -f "$target") == $(readlink -f "$source") ]]; then return; fi
    if [[ -e $target || -L $target ]]; then mv -- "$target" "$target.backup-$(date +%Y%m%d%H%M%S)-$$"; fi
    ln -s -- "$source" "$target"
}
nix-store --add-root "$data_root/dojo/yazi-integration" --indirect --realise "$artifact" >/dev/null
link_file "$artifact/config" "$config_root/xdg-desktop-portal-termfilechooser/config"
link_file "$artifact/portal-routing" "$config_root/xdg-desktop-portal/niri-portals.conf"
link_file "$artifact/portal-routing" "$config_root/xdg-desktop-portal/portals.conf"
link_file "$artifact/dojo-portal" "$data_root/xdg-desktop-portal/portals/dojo.portal"
link_file "$artifact/dojo-yazi.desktop/share/applications/dojo-yazi.desktop" "$data_root/applications/dojo-yazi.desktop"
link_file "$artifact/dbus-filemanager" "$data_root/dbus-1/services/org.freedesktop.FileManager1.service"
link_file "$artifact/systemd-filemanager" "$config_root/systemd/user/dojo-yazi-filemanager.service"
for name in termfilechooser.portal; do
    link_file "$artifact/backend/share/xdg-desktop-portal/portals/$name" "$data_root/xdg-desktop-portal/portals/$name"
done
link_file "$artifact/backend/share/dbus-1/services/org.freedesktop.impl.portal.desktop.termfilechooser.service" "$data_root/dbus-1/services/org.freedesktop.impl.portal.desktop.termfilechooser.service"
link_file "$artifact/backend/lib/systemd/user/xdg-desktop-portal-termfilechooser.service" "$config_root/systemd/user/xdg-desktop-portal-termfilechooser.service"
mkdir -p "$config_root/environment.d"
printf 'GTK_USE_PORTAL=1\n' > "$config_root/environment.d/90-dojo-filechooser.conf"
systemctl --user set-environment GTK_USE_PORTAL=1
dbus-update-activation-environment --systemd GTK_USE_PORTAL=1
systemctl --user daemon-reload
update-desktop-database "$data_root/applications"
xdg-mime default dojo-yazi.desktop inode/directory
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig
systemctl --user start xdg-desktop-portal-termfilechooser.service
systemctl --user restart xdg-desktop-portal.service
systemctl --user start dojo-yazi-filemanager.service
printf 'Yazi folder handler and file-picker portal installed.\n'
