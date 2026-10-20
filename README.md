# soloist-mpris

[![CI](https://github.com/TobiasVeiga00/soloist-mpris/actions/workflows/ci.yml/badge.svg)](https://github.com/TobiasVeiga00/soloist-mpris/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/TobiasVeiga00/soloist-mpris)](https://github.com/TobiasVeiga00/soloist-mpris/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

**Spotify on Linux in about 60 MB of RAM, with your bar, media keys and `playerctl` working.**

Spotify's desktop app is a whole Chromium browser running one web page. In
2026 Spotify released [Soloist](https://developer.spotify.com/blog/2026-08-13-introducing-spotify-soloist),
its official headless player for Linux: a tiny Spotify Connect speaker with no
window and no browser. It sounds great and costs almost nothing, but the rest
of your desktop cannot see it. Your bar shows nothing, play/pause does nothing,
and `playerctl` finds no player.

This kit fixes that. It installs Soloist, keeps it updated, and adds
`soloist-mpris`, a small Rust bridge that publishes Soloist as a standard
[MPRIS](https://specifications.freedesktop.org/mpris-spec/latest/) player.

## What you get

- **Spotify as a background service.** It starts with your session and shows
  up in Spotify Connect as this computer.
- **Your desktop knows what is playing.** Waybar, Quickshell, GNOME, KDE and
  any other MPRIS client get the title, artist, album, cover art and progress.
- **Media keys and `playerctl` work**: play/pause, next, previous, seek,
  volume, shuffle and repeat.
- **15x lighter than the desktop app.** About 60 MB of RAM and 3% of one
  core while playing, against more than 900 MB for Spotify's app
  ([benchmark](#how-light-is-it)).
- **Official playback.** Soloist is Spotify's own player, so it is not affected
  by the [audio key refusals](https://github.com/librespot-org/librespot/issues/1649)
  that stop librespot-based clients on many accounts.
- **Restarts do not stop the music.** When Soloist restarts while it is
  playing (a weekly update, a reinstall), the bridge takes playback back
  within seconds. After a reboot, or when another device had the music, it
  leaves things as they are.
- **No expiry surprises.** Soloist builds stop working after 90 days, so a
  weekly timer fetches the current one from Spotify.

## How light is it?

Same laptop (ThinkPad T480, i5-8350U, Arch Linux), same song, 60 seconds of
playback each, measured with [`bench/measure.sh`](bench/measure.sh):

| | RAM (PSS) | CPU (one core) | Installed size |
| --- | --- | --- | --- |
| **Soloist + soloist-mpris** | **61 MB** | **3%** | **40 MB** |
| Spotify desktop app, window hidden | 922 MB | 10% | 387 MB |
| Spotify desktop app, window open | 943 MB | 27% | 387 MB |

Installed size is the program files on disk (`soloist` and `soloist-mpris`
against Spotify's install directory), caches excluded.

That is **15 times less memory** and less than a third of the CPU, even when
the official app sits hidden in the background. The desktop app is a Chromium
browser showing Spotify's web page, spread over 9 processes.
Here, three small processes play the music and nothing renders at all.

Run the same measurement on your machine while music plays:

```sh
bench/measure.sh '^(soloist|soloist-mpris)$' 60   # this kit
bench/measure.sh '^spotify$' 60                   # the desktop app
```

## Choosing what to play

Soloist is a speaker: it has no library or search of its own. You pick music
with any Spotify Connect controller and play it here:

- the **Spotify app on your phone** (devices icon, then this computer);
- **[open.spotify.com](https://open.spotify.com)** in any browser, closed again
  when the music is going;
- a native client such as [Spotifast](https://github.com/crmne/spotifast), used
  as a remote control;
- the terminal: `soloist ctl play spotify:playlist:...`

Once music plays, everything else comes from your desktop: keys, bar, `playerctl`.

## Requirements

- Linux on x86_64 or arm64 (arm32 works from a clone with Rust), with a
  systemd user session and PipeWire or PulseAudio. Soloist is Linux-only:
  Spotify publishes no Windows or macOS build.
- A Spotify **Premium** account to create the Soloist API key. Once it runs,
  anyone in the house can play on it, Free accounts included.

## Install

```sh
curl -fsSL https://github.com/TobiasVeiga00/soloist-mpris/releases/latest/download/install.sh | bash
```

That is the whole setup. The installer:

1. downloads the bridge for your machine and checks its checksum;
2. downloads Soloist from Spotify;
3. opens the [Spotify for Developers dashboard](https://developer.spotify.com/dashboard)
   and asks for your key: on the **Spotify Soloist API Key** page, accept the
   terms and generate one. It is stored in `~/.config/soloist/env`, readable
   only by you;
4. starts the services and waits while you pair: open Spotify on your phone,
   **on the same network**, tap the devices icon and pick your computer. It
   stays paired from then on.

If a firewall (ufw or firewalld) would hide the computer from your phone, the
installer offers to open it for your local network only while you pair, and
closes it again; if the install is cut short, the next run or the uninstall
closes it. That is the only step that asks for `sudo`; everything else stays in
your home directory.

Prefer to read it first? `curl -fsSLO https://github.com/TobiasVeiga00/soloist-mpris/releases/latest/download/install.sh`,
read it, then `bash install.sh`. From a clone, `./install.sh` does the same and
builds the bridge from source when Rust is installed.

Unattended setup: `SOLOIST_API_KEY=... SOLOIST_DEVICE_NAME="Living room"` before
`bash` skips the questions; pair later from any Spotify app.

## Everyday use

```sh
playerctl -p soloist play-pause           # or your media keys
playerctl -p soloist metadata
systemctl --user restart soloist          # restart the player
journalctl --user -u soloist -u soloist-mpris -f
```

To change the name phones see, add `SOLOIST_DEVICE_NAME=Living room` to
`~/.config/soloist/env` and restart `soloist`.

## Troubleshooting

- **Your phone does not see the computer.** Put both on the same network and
  run the installer again: it walks you through pairing.
- **Spotifast says "Spotify refused the audio key".** Spotifast tried to play
  by itself. Pick this computer in its devices menu instead, and do not use
  **Set up playback here**. If you already did: sign out of Spotifast, quit it,
  set `"playback_authorized": false` in `~/.config/spotifast/settings.json` and
  sign in again.
- **Spotifast shows "Nothing playing" while music plays.** In Spotifast's
  Settings, under Account, choose **Authorize your personal app** again.

## Uninstall

```sh
curl -fsSL https://github.com/TobiasVeiga00/soloist-mpris/releases/latest/download/install.sh | bash -s -- --uninstall
```

It keeps your key and pairing; delete `~/.config/soloist` and
`~/.local/share/soloist` to forget them too.

## How it works

```
Spotify app / web / Spotifast  ->  Spotify Connect  ->  soloist (plays the audio)
                                                          |  local WebSocket (127.0.0.1)
              bar, media keys, playerctl  <-  D-Bus MPRIS  <-  soloist-mpris
```

Soloist runs with its [WebSocket API](https://developer.spotify.com/documentation/soloist)
bound to loopback. `soloist-mpris` follows its events, keeps the playback state
and answers MPRIS calls by sending Soloist the matching commands. It reconnects
whenever Soloist restarts.

## Notes

- Not affiliated with Spotify. Soloist is Spotify's software under
  [Spotify's terms](https://developer.spotify.com/terms); it is never
  redistributed here, only downloaded from Spotify on your machine.
- Your API key belongs to your account; Spotify's terms ask you not to share it.

MIT licensed. See [LICENSE](LICENSE).
