# Contributing

Thanks for helping. soloist-mpris does one job: make Spotify Soloist a good
citizen of the Linux desktop. Changes that keep it small, light and reliable
are the ones that fit.

## Scope

- **In scope:** MPRIS behaviour, reconnection, the installer and services,
  support for more distributions and desktops, documentation.
- **Out of scope:** bundling or redistributing Soloist, sharing API keys,
  anything that works around Spotify's terms, or features Soloist does not
  offer (a library or search belongs in a Spotify Connect controller).

Open an issue before a large change so we can agree on the approach.

## Development

```sh
cargo test                                   # unit tests, no Soloist needed
cargo clippy --all-targets -- -D warnings
cargo fmt --all
bash -n install.sh packaging/soloist-update bench/measure.sh
shellcheck install.sh packaging/soloist-update bench/measure.sh
```

The logic lives in two pure modules with their tests:

- `src/state.rs` turns Soloist's WebSocket events into playback state.
- `src/command.rs` turns MPRIS requests into Soloist commands.

`src/main.rs` only wires them to the socket and to D-Bus. When Soloist sends a
message you have not handled, add it as a JSON fixture in `state.rs` tests,
shaped like the [WebSocket API reference](https://developer.spotify.com/documentation/soloist).

To try a build against your running Soloist:

```sh
systemctl --user stop soloist-mpris
cargo run
playerctl -p soloist metadata
```

## Pull requests

- One change per pull request, with tests for behaviour that can regress.
- [Conventional Commits](https://www.conventionalcommits.org/) for commit
  messages (`fix:`, `feat:`, `docs:` ...).
- Update the README when installation, configuration or behaviour changes.
- Never commit a Soloist binary, an API key, or logs that contain one.
- CI (format, clippy, tests, script syntax) must pass. CodeRabbit reviews
  every pull request; its comments are suggestions, a maintainer decides.

By contributing you agree that your work is released under the MIT License.
