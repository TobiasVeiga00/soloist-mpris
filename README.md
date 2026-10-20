# soloist-mpris

**Spotify on Linux in about 65 MB of RAM, with your bar, media keys and `playerctl` working.**

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
- **Official playback.** Soloist is Spotify's own player, so it is not affected
  by the [audio key refusals](https://github.com/librespot-org/librespot/issues/1649)
  that stop librespot-based clients on many accounts.
- **No expiry surprises.** Soloist builds stop working after 90 days, so a
  weekly timer fetches the current one from Spotify.

## How light is it?

Measured on a ThinkPad T480 (i5-8350U) while playing music:

| Process | RAM (PSS) | CPU |
| --- | --- | --- |
| `soloist` (playing) | 45 MB | about 4% of one core |
| Soloist's crash reporter | 14 MB | idle |
| `soloist-mpris` | 6 MB | idle between events |
| **Total** | **about 65 MB** | |

There is no browser engine anywhere in it, so nothing renders, animates or
runs JavaScript in the background.

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

- Linux on x86_64, arm64 or arm32, with a systemd user session
- PipeWire or PulseAudio
- Spotify **Premium** (Soloist needs it to set up)
- Rust (`cargo`), `curl` and `tar` to install

## Install

```sh
git clone https://github.com/TobiasVeiga00/soloist-mpris
cd soloist-mpris
./install.sh
```

The script builds the bridge, downloads Soloist from Spotify, installs three
user services and asks for your **Soloist API key**. To get one, open the
[Spotify for Developers dashboard](https://developer.spotify.com/dashboard),
go to the **Spotify Soloist API Key** page, accept the terms and generate a
key. The key is stored in `~/.config/soloist/env`, readable only by you.

Everything goes under your home directory; nothing needs root.

### Pair it once

Open Spotify on your phone, **on the same network**, play something, tap the
devices icon and choose your computer (its hostname). Soloist remembers the
login, so from then on it is available from every Spotify app.

If it does not show up, your firewall is blocking the local discovery. Allow
incoming `5353/udp` and the TCP port Soloist listens on (`ss -tlnp | grep soloist`)
from your local network, pair, then remove the rules.

## Everyday use

```sh
playerctl -p soloist play-pause           # or your media keys
playerctl -p soloist metadata
systemctl --user restart soloist          # restart the player
journalctl --user -u soloist -u soloist-mpris -f
```

To change the name phones see, add `SOLOIST_DEVICE_NAME=Living room` to
`~/.config/soloist/env` and restart `soloist`.

## Uninstall

```sh
./install.sh --uninstall
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
