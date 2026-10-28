#!/usr/bin/env bash
# Put the Rex package token where Nix can use it: a private netrc file that nix.conf points at.
#
# Rex's package server (the `rex` input in devenv.yaml) answers 401 without the token, so every
# devenv evaluation on a machine needs it. The token is read from 1Password, never stored in the
# repository. REX_TOKEN_REF overrides the reference: op://vault/item/field, env:NAME or file:/path.
set -euo pipefail

ref=${REX_TOKEN_REF:-op://Personal/Rex Token/credential}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
netrc=$config_home/nix/rex-netrc
nix_conf=$config_home/nix/nix.conf
probe=https://pkgs.superlogical.com/rex/nix/preview.tar.gz

case $ref in
    op://*) token=$(op read --no-newline "$ref") ;;
    env:*) name=${ref#env:}; token=${!name:-} ;;
    file:*) token=$(<"${ref#file:}") ;;
    *) echo "Unsupported reference: $ref" >&2; exit 1 ;;
esac
[[ -n $token ]] || { echo "No token at $ref" >&2; exit 1; }

mkdir -p "$(dirname "$netrc")"
(umask 077; printf 'machine pkgs.superlogical.com login rex password %s\n' "$token" >"$netrc.new")
code=$(curl -sS -o /dev/null -w '%{http_code}' -I --max-time 20 --netrc-file "$netrc.new" "$probe" || true)
if [[ $code != 200 && $code != 302 ]]; then
    rm -f "$netrc.new"
    echo "The package server answered $code; the token at $ref is wrong or not enabled." >&2
    exit 1
fi
mv -f "$netrc.new" "$netrc"
touch "$nix_conf"
grep -qxF "netrc-file = $netrc" "$nix_conf" || printf 'netrc-file = %s\n' "$netrc" >>"$nix_conf"
echo "Rex package token installed at $netrc"
