//! MPRIS requests as Soloist WebSocket commands.

use serde_json::{Value, json};

use crate::state::{Repeat, State, Status};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    /// Absolute position, in milliseconds.
    SeekTo(i64),
    /// Relative offset, in milliseconds.
    SeekBy(i64),
    /// 0.0 to 1.0.
    Volume(f64),
    Shuffle(bool),
    Repeat(Repeat),
}

/// The commands that carry out `request` given what is playing at `now_ms`.
pub fn commands(request: Request, state: &State, now_ms: i64) -> Vec<Value> {
    // Every control command needs a Spotify Connect session.
    if !state.logged_in {
        return Vec::new();
    }
    let duration = state.track.as_ref().and_then(|track| track.duration_ms);
    let seek = |position_ms: i64| match duration {
        // MPRIS: seeking past the end of the track acts like Next.
        Some(duration) if position_ms > duration => command("skip_next"),
        _ => json!({ "type": "command", "command": "seek", "position_ms": position_ms.max(0) }),
    };
    let set = |name: &str, enabled: bool| json!({ "type": "command", "command": name, "enabled": enabled });
    match request {
        Request::Play => vec![command("play")],
        Request::Pause | Request::Stop => vec![command("pause")],
        Request::PlayPause if state.status == Status::Playing => vec![command("pause")],
        Request::PlayPause => vec![command("play")],
        Request::Next => vec![command("skip_next")],
        Request::Previous => vec![command("skip_prev")],
        Request::SeekTo(position_ms) => vec![seek(position_ms)],
        Request::SeekBy(offset_ms) => vec![seek(state.position_ms(now_ms) + offset_ms)],
        Request::Volume(volume) => {
            let percent = (volume.clamp(0.0, 1.0) * 100.0).round() as u64;
            vec![json!({ "type": "command", "command": "set_volume", "volume": percent })]
        }
        Request::Shuffle(enabled) => vec![set("set_shuffle", enabled)],
        // Two commands, in the order Soloist's reference gives for each mode.
        Request::Repeat(Repeat::Off) => vec![
            set("set_repeat_track", false),
            set("set_repeat_context", false),
        ],
        Request::Repeat(Repeat::Context) => vec![
            set("set_repeat_track", false),
            set("set_repeat_context", true),
        ],
        Request::Repeat(Repeat::Track) => vec![
            set("set_repeat_context", false),
            set("set_repeat_track", true),
        ],
    }
}

fn command(name: &str) -> Value {
    json!({ "type": "command", "command": name })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Anchor, Track};

    fn command(name: &str) -> Value {
        json!({ "type": "command", "command": name })
    }

    fn playing() -> State {
        State {
            logged_in: true,
            status: Status::Playing,
            track: Some(Track {
                duration_ms: Some(200_000),
                ..Track::default()
            }),
            anchor: Anchor {
                position_ms: 60_000,
                timestamp_ms: 0,
                speed: 1.0,
            },
            ..State::default()
        }
    }

    #[test]
    fn transport_requests_map_to_their_commands() {
        let state = playing();
        assert_eq!(commands(Request::Play, &state, 0), vec![command("play")]);
        assert_eq!(commands(Request::Pause, &state, 0), vec![command("pause")]);
        assert_eq!(commands(Request::Stop, &state, 0), vec![command("pause")]);
        assert_eq!(
            commands(Request::Next, &state, 0),
            vec![command("skip_next")]
        );
        assert_eq!(
            commands(Request::Previous, &state, 0),
            vec![command("skip_prev")]
        );
    }

    #[test]
    fn play_pause_follows_the_status() {
        let mut state = playing();
        assert_eq!(
            commands(Request::PlayPause, &state, 0),
            vec![command("pause")]
        );
        state.status = Status::Paused;
        assert_eq!(
            commands(Request::PlayPause, &state, 0),
            vec![command("play")]
        );
    }

    #[test]
    fn seeking_is_relative_to_the_live_position_and_clamped() {
        let state = playing();
        let seek = |ms: i64| json!({ "type": "command", "command": "seek", "position_ms": ms });
        assert_eq!(
            commands(Request::SeekBy(10_000), &state, 5_000),
            vec![seek(75_000)]
        );
        assert_eq!(commands(Request::SeekBy(-90_000), &state, 0), vec![seek(0)]);
        assert_eq!(
            commands(Request::SeekTo(30_000), &state, 0),
            vec![seek(30_000)]
        );
        // MPRIS: seeking past the end acts like Next.
        assert_eq!(
            commands(Request::SeekBy(500_000), &state, 0),
            vec![command("skip_next")]
        );
    }

    #[test]
    fn volume_is_scaled_to_a_percentage() {
        let state = playing();
        let volume = |v: u64| json!({ "type": "command", "command": "set_volume", "volume": v });
        assert_eq!(
            commands(Request::Volume(0.426), &state, 0),
            vec![volume(43)]
        );
        assert_eq!(commands(Request::Volume(1.7), &state, 0), vec![volume(100)]);
        assert_eq!(commands(Request::Volume(-0.2), &state, 0), vec![volume(0)]);
    }

    #[test]
    fn shuffle_and_repeat_use_the_documented_command_pairs() {
        let state = playing();
        let set =
            |name: &str, on: bool| json!({ "type": "command", "command": name, "enabled": on });
        assert_eq!(
            commands(Request::Shuffle(true), &state, 0),
            vec![set("set_shuffle", true)]
        );
        assert_eq!(
            commands(Request::Repeat(Repeat::Off), &state, 0),
            vec![
                set("set_repeat_track", false),
                set("set_repeat_context", false)
            ]
        );
        assert_eq!(
            commands(Request::Repeat(Repeat::Context), &state, 0),
            vec![
                set("set_repeat_track", false),
                set("set_repeat_context", true)
            ]
        );
        assert_eq!(
            commands(Request::Repeat(Repeat::Track), &state, 0),
            vec![
                set("set_repeat_context", false),
                set("set_repeat_track", true)
            ]
        );
    }

    #[test]
    fn nothing_is_sent_before_login() {
        let state = State::default();
        assert!(commands(Request::Play, &state, 0).is_empty());
    }
}
