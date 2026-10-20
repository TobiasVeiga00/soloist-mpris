#!/bin/bash
# install.sh: set up Spotify Soloist with soloist-mpris for this user.
#
#   curl -fsSL https://github.com/TobiasVeiga00/soloist-mpris/releases/latest/download/install.sh | bash
#   ./install.sh                  the same, from a clone (builds the bridge when Rust is there)
#   ./install.sh --uninstall      stop and remove everything this script installed
#
# It installs the bridge, downloads Soloist from Spotify, asks for your API
# key, starts the user services and walks you through pairing. Everything
# goes under your home directory. Root is only ever asked for, with your
# consent, to open the firewall while you pair.
#
# Environment:
#   SOLOIST_API_KEY=...       use this key and ask nothing (unattended setup)
#   SOLOIST_DEVICE_NAME=...   the name phones show (default: the hostname)
#   SOLOIST_MPRIS_RELEASE_URL where to download the bridge (default: latest release)
set -euo pipefail

repo=TobiasVeiga00/soloist-mpris
release=${SOLOIST_MPRIS_RELEASE_URL:-https://github.com/$repo/releases/latest/download}
dashboard=https://developer.spotify.com/dashboard
bin="$HOME/.local/bin"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
config="${XDG_CONFIG_HOME:-$HOME/.config}/soloist"
state="${XDG_STATE_HOME:-$HOME/.local/state}/soloist-mpris"
unit_files=(soloist.service soloist-mpris.service soloist-update.service soloist-update.timer)
pair_minutes=5
# Covers the service's network wait (up to two minutes) plus Soloist's start.
settle_seconds=150
work="" since=0

say() { printf '\033[1m%s\033[0m\n' "$*"; }
note() { printf '  %s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

# Piped into bash, stdin is this script, so questions read the terminal.
# The whole install runs from main(), called on the last line: bash has read
# every line before anything runs, so a cut download runs nothing and a child
# that reads stdin finds nothing left to eat.
interactive() { [[ -r /dev/tty ]] && { : </dev/tty; } 2>/dev/null; }
ask() { local reply=""; read -r -p "$1" reply </dev/tty || true; printf '%s' "$reply"; }
ask_secret() { local reply=""; read -rs -p "$1" reply </dev/tty || true; echo >/dev/tty; printf '%s' "$reply"; }

set_env() { # name value: replace or add one line of the env file
  local rest
  rest=$(grep -v "^$1=" "$config/env" 2>/dev/null || true)
  (umask 077 && printf '%s%s=%s\n' "${rest:+$rest$'\n'}" "$1" "$2" >"$config/env")
}

env_value() { sed -n "s/^$1=//p" "$config/env" 2>/dev/null | tail -n 1; }

ask_key() {
  interactive || die "set SOLOIST_API_KEY to install without a terminal"
  note "Soloist needs an API key from your Spotify Premium account:"
  note "open $dashboard, go to the Spotify Soloist API Key page,"
  note "accept the terms and generate a key."
  if command -v xdg-open >/dev/null && [[ -n ${WAYLAND_DISPLAY:-}${DISPLAY:-} ]]; then
    xdg-open "$dashboard" >/dev/null 2>&1 || true
    note "(The dashboard just opened in your browser.)"
  fi
  local key
  key=$(ask_secret "  Paste the key (input hidden): ")
  key=${key//[[:space:]]/}
  [[ -n $key ]] || die "no key given; run the installer again when you have it"
  set_env SOLOIST_API_KEY "$key"
}

# Status checks capture output first: with pipefail, `cmd | grep -q` can fail
# when grep stops reading early. Soloist has no status field for "waiting for
# login" or a rejected key, so these read its lines; the Soloist lines
# workflow checks every week that Spotify's current build still prints them.
logged_in() {
  local status
  status=$("$bin/soloist" ctl status 2>/dev/null) || true
  [[ $status == *"logged in: yes"* ]]
}
logged_since() { # text: Soloist logged it since the service last (re)started
  local log
  log=$(journalctl --user -u soloist --since "@$since" --no-pager -o cat 2>/dev/null) || true
  [[ $log == *"$1"* ]]
}
key_rejected() { logged_since 'login failed'; }

restart_soloist() {
  since=$(($(date +%s) - 1))
  systemctl --user restart soloist.service
}

# Asks for a new key after Spotify rejects one; the third rejection ends the
# install. Soloist listens on a new port after the restart, so open firewall
# rules follow it.
rejections=0
max_rejections=3
retry_key() {
  rejections=$((rejections + 1))
  if ((rejections >= max_rejections)); then
    close_firewall
    die "Spotify rejected the API key $max_rejections times; generate a new one at $dashboard and run the installer again"
  fi
  note "Spotify rejected the API key."
  ask_key
  local reopen=""
  [[ -s $opened ]] && reopen=1
  close_firewall
  restart_soloist
  if [[ -n $reopen ]]; then
    { find_port && open_firewall; } || note "Could not open the firewall again; your phone may not find this computer."
  fi
}

# Soloist's discovery port, which changes on every start.
find_port() {
  local i
  for ((i = 0; i < 10; i++)); do
    port=$(ss -tlnpH 2>/dev/null | awk '/"soloist"/ && $4 !~ /^127\./ { n = split($4, a, ":"); print a[n]; exit }') || true
    [[ -n $port ]] && return 0
    sleep 1
  done
  return 1
}

# Discovery needs mDNS (5353/udp) and Soloist's own TCP port, from the local
# network only. Every rule this script added is taken out again, even if a
# later one failed; a rule that was already there is left alone.
#
# Each added rule is recorded in a file (tool, firewalld zone or -, rule) so
# that one left open by a killed installer is closed by the next run or by
# --uninstall.
firewall="" zone="-" dev="" subnet="" port=""
opened="$state/firewall-rules"
open_firewall() {
  local rules=() rule
  case $firewall in
    ufw) rules=("allow in from $subnet to any port 5353 proto udp"
                "allow in from $subnet to any port $port proto tcp") ;;
    firewalld)
      # Rules only apply in the zone of the interface the phone talks to.
      zone=$(sudo firewall-cmd --get-zone-of-interface="$dev" 2>/dev/null) ||
        zone=$(sudo firewall-cmd --get-default-zone) || return 1
      rules=("rule family=ipv4 source address=$subnet port port=5353 protocol=udp accept"
             "rule family=ipv4 source address=$subnet port port=$port protocol=tcp accept") ;;
  esac
  mkdir -p "$state"
  local out
  for rule in "${rules[@]}"; do
    # shellcheck disable=SC2086 # the rule is ufw's own words
    case $firewall in
      ufw)
        out=$(sudo ufw $rule 2>&1) || return 1
        [[ $out != *Skipping* ]] || continue ;;
      firewalld)
        ! sudo firewall-cmd --quiet --zone="$zone" --query-rich-rule="$rule" 2>/dev/null || continue
        sudo firewall-cmd --quiet --zone="$zone" --add-rich-rule="$rule" || return 1 ;;
    esac
    printf '%s\t%s\t%s\n' "$firewall" "$zone" "$rule" >>"$opened"
  done
}
close_firewall() { # [message]
  [[ -s $opened ]] || return 0
  note "${1:-Closing the firewall again.}"
  local tool rule_zone rule hint left=""
  while IFS=$'\t' read -r -u 3 tool rule_zone rule; do
    # shellcheck disable=SC2086 # the rule is ufw's own words
    case $tool in
      ufw)
        if sudo ufw delete $rule >/dev/null; then continue; fi
        hint="sudo ufw delete $rule" ;;
      firewalld)
        if sudo firewall-cmd --quiet --zone="$rule_zone" --remove-rich-rule="$rule"; then continue; fi
        hint="sudo firewall-cmd --zone=$rule_zone --remove-rich-rule='$rule'" ;;
      *) continue ;;
    esac
    note "Could not remove a firewall rule; remove it with: $hint"
    left+="$tool"$'\t'"$rule_zone"$'\t'"$rule"$'\n'
  done 3<"$opened"
  # A rule that could not be removed stays recorded for the next run.
  if [[ -n $left ]]; then
    printf '%s' "$left" >"$opened"
  else
    rm -f -- "$opened"
  fi
}

download() { # file of the release, into the work directory
  curl --fail --location --silent --show-error -o "$work/$1" "$release/$1" ||
    die "could not download $1 from $release; a release published in the last few minutes may still be uploading its files, so try again in a few minutes"
}

cleanup() {
  close_firewall
  [[ -z $work ]] || rm -rf -- "${work:?}"
}

main() {
  # A clone has the packaging next to this script; a piped install has neither.
  local here=""
  if [[ -n ${BASH_SOURCE[0]:-} && -f ${BASH_SOURCE[0]} ]]; then
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    [[ -f $here/packaging/systemd/soloist.service ]] || here=""
  fi

  close_firewall "An interrupted install left the firewall open while pairing; closing it."

  if [[ ${1:-} == --uninstall ]]; then
    local unit
    systemctl --user disable --now --quiet soloist-update.timer soloist-mpris.service soloist.service 2>/dev/null || true
    for unit in "${unit_files[@]}"; do rm -f -- "${units:?}/$unit"; done
    systemctl --user daemon-reload
    rm -f -- "${bin:?}/soloist" "${bin:?}/soloist-mpris" "${bin:?}/soloist-update"
    say "Removed. Your API key and pairing are kept in $config and ~/.local/share/soloist; delete them to forget this device."
    return 0
  fi

  [[ $(uname -s) == Linux ]] || die "Soloist only runs on Linux"
  local tool
  for tool in curl tar sha256sum systemctl; do
    command -v "$tool" >/dev/null || die "$tool is required"
  done
  systemctl --user show-environment >/dev/null 2>&1 || die "no systemd user session; log in to a desktop session and run it again"
  command -v pactl >/dev/null || command -v pw-cli >/dev/null ||
    note "No PipeWire or PulseAudio tools found; Soloist needs one of them to play sound."

  work=$(mktemp -d)
  trap cleanup EXIT

  # 1. The bridge, and the service files that go with it.
  local packaging archive=""
  if [[ -n $here ]] && command -v cargo >/dev/null; then
    say "1/4 Building soloist-mpris"
    cargo install --locked --quiet --force --path "$here" --root "$HOME/.local"
    packaging="$here/packaging"
  else
    local arch
    arch=$(uname -m)
    case $arch in
      x86_64 | aarch64) ;;
      *) die "no prebuilt soloist-mpris for $arch; clone the repository, install Rust (https://rustup.rs) and run ./install.sh" ;;
    esac
    say "1/4 Downloading soloist-mpris ($arch)"
    archive="soloist-mpris-$arch-linux.tar.gz"
    download "$archive"
    download "$archive.sha256"
    (cd "$work" && sha256sum --check --quiet "$archive.sha256") || die "checksum mismatch for $archive"
    tar -xzf "$work/$archive" -C "$work"
    install -Dm 755 "$work/soloist-mpris" "$bin/soloist-mpris"
    packaging="${here:+$here/packaging}"
    packaging="${packaging:-$work/packaging}"
  fi
  [[ -f $packaging/systemd/soloist.service ]] ||
    die "$archive from $release has no service files; releases before v0.2.0 do not carry them"

  # 2. Soloist itself, always from Spotify.
  say "2/4 Downloading Spotify Soloist from Spotify"
  install -Dm 755 "$packaging/soloist-update" "$bin/soloist-update"
  "$bin/soloist-update"

  # 3. The API key and the services.
  say "3/4 Setting up"
  mkdir -p "$config" "$units"
  chmod 700 "$config"
  # A key from the environment means nobody is there to answer questions.
  local unattended=""
  if [[ -n ${SOLOIST_API_KEY:-} ]]; then
    unattended=1
    set_env SOLOIST_API_KEY "${SOLOIST_API_KEY//[[:space:]]/}"
  elif [[ -z $(env_value SOLOIST_API_KEY) ]]; then
    ask_key
  else
    note "Using the API key already in $config/env"
  fi
  [[ -z ${SOLOIST_DEVICE_NAME:-} ]] || set_env SOLOIST_DEVICE_NAME "$SOLOIST_DEVICE_NAME"
  interactive || unattended=1

  local unit
  for unit in "${unit_files[@]}"; do install -m 644 "$packaging/systemd/$unit" "$units/$unit"; done
  systemctl --user daemon-reload
  since=$(($(date +%s) - 1))
  systemctl --user enable --now --quiet soloist-update.timer soloist.service soloist-mpris.service
  # A unit file that changed on a reinstall only applies after a restart.
  systemctl --user try-restart soloist.service soloist-mpris.service

  local name pair_hint
  name=$(env_value SOLOIST_DEVICE_NAME)
  name=${name:-$(hostname)}
  pair_hint="open Spotify on your phone, on the same network, play something, tap the devices icon and choose \"$name\""

  # 4. Pairing: Soloist logs in when a Spotify app on the same network picks it.
  say "4/4 Pairing"
  local i
  for ((i = 0; i < settle_seconds; i++)); do
    if logged_in || logged_since 'waiting for login' || key_rejected; then break; fi
    sleep 1
  done
  if logged_in; then
    say "Done. \"$name\" is already paired and ready in every Spotify app."
    return 0
  fi
  if key_rejected; then
    [[ -z $unattended ]] || die "Spotify rejected the API key in $config/env"
    retry_key
  fi
  if [[ -n $unattended ]]; then
    say "Done. Pair it once: $pair_hint."
    return 0
  fi

  systemctl is-active --quiet ufw 2>/dev/null && firewall=ufw
  systemctl is-active --quiet firewalld 2>/dev/null && firewall=firewalld
  if [[ -n $firewall ]]; then
    local reply
    dev=$(ip -4 route show default 2>/dev/null | awk '{print $5; exit}') || true
    subnet=$(ip -4 route show dev "$dev" proto kernel scope link 2>/dev/null | awk '{print $1; exit}') || true
    if [[ -n $subnet ]] && find_port; then
      note "Your firewall ($firewall) blocks the phone from finding this computer."
      reply=$(ask "  Open it for $subnet only while you pair, then close it? [Y/n] ")
      if [[ ! $reply =~ ^[Nn] ]]; then
        open_firewall || note "Could not open the firewall; your phone may not find this computer."
      fi
    fi
  fi

  local deadline
  deadline=$(($(date +%s) + pair_minutes * 60))
  note "Now $pair_hint. Waiting up to $pair_minutes minutes..."
  while (($(date +%s) < deadline)); do
    if logged_in; then
      close_firewall
      say "Paired. \"$name\" is ready in every Spotify app, and your media keys and bar control it."
      return 0
    fi
    if key_rejected; then
      retry_key
      deadline=$(($(date +%s) + pair_minutes * 60))
      note "Now $pair_hint again. Waiting up to $pair_minutes minutes..."
      continue
    fi
    sleep 2
  done
  close_firewall
  if ! systemctl --user is-active --quiet soloist.service; then
    die "Soloist is not running; see journalctl --user -u soloist"
  fi
  say "Not paired yet. Soloist keeps running: pick \"$name\" on your phone whenever you like,"
  say "or run the installer again to be walked through it."
}

main "$@"
