//! Taking playback back after Soloist restarts.
//!
//! A Soloist restart (a weekly update, a reinstall, `systemctl restart`)
//! drops it as the active Spotify Connect device: the music stops, and
//! another device, such as a controller app's own player, may take the queue.
//! Whenever the bridge loses Soloist it saves whether this speaker had
//! playback; when Soloist comes back in the same boot and soon after, the
//! bridge takes playback back. A reboot or a long gap never starts music on
//! its own.
//!
//! Pure: the bridge supplies the boot id, the clock and the file.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::{State, Status};

/// How soon Soloist must be back for playback to be taken back. A restart
/// takes seconds; the service's network wait can add up to two minutes.
pub const WINDOW_MS: i64 = 150_000;

/// What this speaker was doing when the bridge lost Soloist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub boot_id: String,
    pub at_ms: i64,
    /// This speaker had playback (it was the active device with a track).
    pub active: bool,
    /// And it was playing, not paused.
    pub playing: bool,
}

impl Saved {
    /// The record to save for `state`; `None` until Soloist has said who
    /// has playback, so a bridge that never heard from it saves nothing.
    pub fn of(state: &State, boot_id: &str, now_ms: i64) -> Option<Saved> {
        if !state.logged_in {
            return None;
        }
        let active = !state.inactive && state.track.is_some();
        Some(Saved {
            boot_id: boot_id.to_owned(),
            at_ms: now_ms,
            active,
            playing: active && state.status == Status::Playing,
        })
    }
}

/// The commands that take playback back, given what was saved and the
/// first snapshot of the new connection.
pub fn commands(saved: Option<&Saved>, state: &State, boot_id: &str, now_ms: i64) -> Vec<Value> {
    let Some(saved) = saved else {
        return Vec::new();
    };
    let recent = (0..=WINDOW_MS).contains(&(now_ms - saved.at_ms));
    // An unknown boot id cannot tell a restart from a reboot: never resume.
    let same_boot = !boot_id.is_empty() && saved.boot_id == boot_id;
    if !saved.active || !same_boot || !recent || !state.logged_in || !state.inactive {
        return Vec::new();
    }
    let command = |name: &str| json!({ "type": "command", "command": name });
    let mut commands = vec![command("activate")];
    if saved.playing {
        commands.push(command("play"));
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Track;

    const BOOT: &str = "15f05f24-2359-4531-a60b-e1e49f4aa04d";

    fn command(name: &str) -> Value {
        json!({ "type": "command", "command": name })
    }

    fn playing_here() -> State {
        State {
            logged_in: true,
            status: Status::Playing,
            track: Some(Track::default()),
            ..State::default()
        }
    }

    fn back_inactive() -> State {
        State {
            logged_in: true,
            inactive: true,
            status: Status::Stopped,
            track: Some(Track::default()),
            ..State::default()
        }
    }

    fn saved(active: bool, playing: bool, at_ms: i64) -> Saved {
        Saved {
            boot_id: BOOT.into(),
            at_ms,
            active,
            playing,
        }
    }

    #[test]
    fn what_is_saved_follows_who_has_playback() {
        assert_eq!(
            Saved::of(&playing_here(), BOOT, 7),
            Some(saved(true, true, 7))
        );
        let paused = State {
            status: Status::Paused,
            ..playing_here()
        };
        assert_eq!(Saved::of(&paused, BOOT, 7), Some(saved(true, false, 7)));
        assert_eq!(
            Saved::of(&back_inactive(), BOOT, 7),
            Some(saved(false, false, 7))
        );
        let no_track = State {
            track: None,
            ..playing_here()
        };
        assert_eq!(Saved::of(&no_track, BOOT, 7), Some(saved(false, false, 7)));
        assert_eq!(
            Saved::of(&State::default(), BOOT, 7),
            None,
            "Soloist never answered"
        );
    }

    #[test]
    fn a_quick_restart_takes_playback_back_and_plays_on() {
        let before = saved(true, true, 1_000_000);
        assert_eq!(
            commands(Some(&before), &back_inactive(), BOOT, 1_010_000),
            vec![command("activate"), command("play")]
        );
    }

    #[test]
    fn a_paused_speaker_comes_back_paused() {
        let before = saved(true, false, 1_000_000);
        assert_eq!(
            commands(Some(&before), &back_inactive(), BOOT, 1_010_000),
            vec![command("activate")]
        );
    }

    #[test]
    fn nothing_is_taken_without_reason() {
        let now = 1_010_000;
        let elsewhere = saved(false, false, 1_000_000);
        assert!(
            commands(Some(&elsewhere), &back_inactive(), BOOT, now).is_empty(),
            "another device had playback"
        );
        let before = saved(true, true, 1_000_000);
        let reboot = Saved {
            boot_id: "another-boot".into(),
            ..before.clone()
        };
        assert!(
            commands(Some(&reboot), &back_inactive(), BOOT, now).is_empty(),
            "a reboot"
        );
        assert!(
            commands(
                Some(&before),
                &back_inactive(),
                BOOT,
                1_000_000 + WINDOW_MS + 1
            )
            .is_empty(),
            "too long ago"
        );
        assert!(
            commands(Some(&before), &playing_here(), BOOT, now).is_empty(),
            "it already has playback"
        );
        let logged_out = State {
            logged_in: false,
            ..back_inactive()
        };
        assert!(
            commands(Some(&before), &logged_out, BOOT, now).is_empty(),
            "not logged in"
        );
        assert!(
            commands(None, &back_inactive(), BOOT, now).is_empty(),
            "nothing saved"
        );
        let unknown = Saved {
            boot_id: String::new(),
            ..before.clone()
        };
        assert!(
            commands(Some(&unknown), &back_inactive(), "", now).is_empty(),
            "without a boot id a reboot cannot be told apart"
        );
    }
}
