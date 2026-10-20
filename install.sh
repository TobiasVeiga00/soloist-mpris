#!/bin/bash
# install.sh: set up Spotify Soloist with soloist-mpris for this user.
#
#   ./install.sh               install the bridge, download Soloist, ask for the API key, start
#   ./install.sh --uninstall   stop and remove everything this script installed
#
# With Rust installed the bridge is built from this checkout; without it, the
# prebuilt binary for this architecture comes from the latest GitHub release.
# Everything goes under your home directory; nothing needs root.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
config="${XDG_CONFIG_HOME:-$HOME/.config}/soloist"
unit_files=(soloist.service soloist-mpris.service soloist-update.service soloist-update.timer)

say() { printf '\033[1m%s\033[0m\n' "$*"; }

if [[ ${1:-} == --uninstall ]]; then
  systemctl --user disable --now soloist-update.timer soloist-mpris.service soloist.service 2>/dev/null || true
  for unit in "${unit_files[@]}"; do rm -f "$units/$unit"; done
  systemctl --user daemon-reload
  rm -f "$bin/soloist" "$bin/soloist-mpris" "$bin/soloist-update"
  say "Removed. Your API key and pairing are kept in $config and ~/.local/share/soloist; delete them to forget this device."
  exit 0
fi

for tool in curl tar systemctl; do
  command -v "$tool" >/dev/null || { echo "install.sh: $tool is required" >&2; exit 1; }
done

if command -v cargo >/dev/null; then
  say "1/4 Building soloist-mpris"
  cargo install --locked --quiet --force --path "$here" --root "$HOME/.local"
else
  arch=$(uname -m)
  case $arch in
    x86_64 | aarch64) ;;
    *) echo "install.sh: no prebuilt soloist-mpris for $arch; install Rust (https://rustup.rs) and run again" >&2; exit 1 ;;
  esac
  say "1/4 Downloading soloist-mpris ($arch)"
  release="https://github.com/TobiasVeiga00/soloist-mpris/releases/latest/download"
  archive="soloist-mpris-$arch-linux.tar.gz"
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  curl --fail --location --silent --show-error -o "$work/$archive" "$release/$archive"
  curl --fail --location --silent --show-error -o "$work/$archive.sha256" "$release/$archive.sha256"
  (cd "$work" && sha256sum --check --quiet "$archive.sha256")
  tar -xzf "$work/$archive" -C "$work" soloist-mpris
  install -Dm 755 "$work/soloist-mpris" "$bin/soloist-mpris"
fi

say "2/4 Downloading Spotify Soloist from Spotify"
install -Dm 755 "$here/packaging/soloist-update" "$bin/soloist-update"
"$bin/soloist-update"

say "3/4 Installing the user services"
mkdir -p "$units"
for unit in "${unit_files[@]}"; do install -m 644 "$here/packaging/systemd/$unit" "$units/$unit"; done
systemctl --user daemon-reload

if [[ ! -s $config/env ]]; then
  say "4/4 Soloist API key"
  echo "Open https://developer.spotify.com/dashboard (Premium account), go to the"
  echo "Spotify Soloist API Key page, accept the terms and generate a key."
  read -rsp "Paste the key (input hidden): " key
  echo
  [[ -n $key ]] || { echo "install.sh: no key given; run ./install.sh again" >&2; exit 1; }
  mkdir -p "$config"
  chmod 700 "$config"
  (umask 077 && printf 'SOLOIST_API_KEY=%s\n' "$key" >"$config/env")
else
  say "4/4 Soloist API key already in $config/env"
fi

systemctl --user enable --now soloist-update.timer soloist.service soloist-mpris.service
# A unit file that changed on a reinstall only applies after a restart.
systemctl --user try-restart soloist.service
name=$(sed -n 's/^SOLOIST_DEVICE_NAME=//p' "$config/env" | tail -n 1)
name=${name:-$(hostname)}
say "Done. Soloist is running as \"$name\"."
echo "First time only: open Spotify on your phone, on the same network, tap the"
echo "devices icon and choose \"$name\". It stays paired after that."
