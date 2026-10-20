//! What Soloist is playing, kept from its WebSocket events.
//!
//! Pure: no I/O and no clock. The bridge feeds every event to
//! [`State::apply`] and turns the returned [`Changes`] into MPRIS signals.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    Context,
    Track,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub art_url: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Where playback was at a server time, and how fast it moves from there.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Anchor {
    pub position_ms: i64,
    pub timestamp_ms: i64,
    pub speed: f64,
}

/// Which controls Soloist offers right now (`available_actions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Actions {
    pub play: bool,
    pub pause: bool,
    pub next: bool,
    pub previous: bool,
    pub seek: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub logged_in: bool,
    pub status: Status,
    pub track: Option<Track>,
    pub anchor: Anchor,
    /// 0 to 100.
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub actions: Actions,
    /// Another Spotify Connect device has playback; nothing plays here.
    pub inactive: bool,
}

/// What one event changed, so only those MPRIS properties are announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Changes {
    pub metadata: bool,
    pub status: bool,
    pub volume: bool,
    pub options: bool,
    pub actions: bool,
    /// The position jumped instead of moving on at its speed.
    pub seeked: bool,
}

/// A position that drifts less than this from where it should be is not a seek.
const SEEK_TOLERANCE_MS: i64 = 1000;

impl State {
    /// Applies one server message and says what it changed.
    pub fn apply(&mut self, event: &Value) -> Changes {
        let none = Changes::default();
        match event["type"].as_str() {
            Some("playback_state") => {
                self.logged_in = true;
                self.inactive = event["is_active"].as_bool() == Some(false);
                // An inactive device's snapshot describes playback elsewhere.
                self.status = match self.inactive {
                    true => Status::Stopped,
                    false => status(&event["status"]).unwrap_or_default(),
                };
                self.track = track(&event["item"]);
                self.anchor = anchor(&event["position"]).unwrap_or_default();
                if let Some(volume) = volume(&event["volume"]) {
                    self.volume = volume;
                }
                self.apply_options(&event["options"]);
                self.actions = actions(&event["available_actions"]);
                Changes {
                    metadata: true,
                    status: true,
                    volume: true,
                    options: true,
                    actions: true,
                    seeked: true,
                }
            }
            Some("track_changed") => {
                self.track = track(&event["item"]);
                Changes {
                    metadata: true,
                    ..none
                }
            }
            // While another device has playback, its status is not ours.
            Some("playback_changed") if self.inactive => none,
            Some("playback_changed") => match status(&event["status"]) {
                Some(status) => {
                    self.status = status;
                    Changes {
                        status: true,
                        ..none
                    }
                }
                None => none,
            },
            Some("volume_changed") => match volume(&event["volume"]) {
                Some(volume) => {
                    self.volume = volume;
                    Changes {
                        volume: true,
                        ..none
                    }
                }
                None => none,
            },
            Some("options_changed") => {
                self.apply_options(&event["options"]);
                Changes {
                    options: true,
                    ..none
                }
            }
            Some("position_sync") => match anchor(&event["position"]) {
                Some(next) => {
                    let expected = self.position_ms(next.timestamp_ms);
                    self.anchor = next;
                    Changes {
                        seeked: (next.position_ms - expected).abs() > SEEK_TOLERANCE_MS,
                        ..none
                    }
                }
                None => none,
            },
            Some("device_changed") => match event["is_active"].as_bool() {
                Some(false) => {
                    // Playback moved to another device: the last track stays
                    // on the tile, stopped, until it comes back.
                    self.inactive = true;
                    self.status = Status::Stopped;
                    Changes {
                        status: true,
                        ..none
                    }
                }
                Some(true) => {
                    // The playback_changed that follows says how it plays.
                    self.inactive = false;
                    none
                }
                None => none,
            },
            Some("auth_state") => {
                self.logged_in = event["logged_in"].as_bool().unwrap_or(false);
                if self.logged_in {
                    // A playback_state snapshot follows a login.
                    return none;
                }
                self.track = None;
                self.status = Status::Stopped;
                self.anchor = Anchor::default();
                Changes {
                    metadata: true,
                    status: true,
                    ..none
                }
            }
            _ => none,
        }
    }

    /// The playback position at `now_ms` (Unix epoch milliseconds).
    pub fn position_ms(&self, now_ms: i64) -> i64 {
        let Anchor {
            position_ms,
            timestamp_ms,
            speed,
        } = self.anchor;
        let moved = if speed == 0.0 {
            0
        } else {
            ((now_ms - timestamp_ms) as f64 * speed) as i64
        };
        let position = (position_ms + moved).max(0);
        match self.track.as_ref().and_then(|track| track.duration_ms) {
            Some(duration) => position.min(duration),
            None => position,
        }
    }

    fn apply_options(&mut self, options: &Value) {
        if let Some(shuffle) = options["shuffle"].as_bool() {
            self.shuffle = shuffle;
        }
        self.repeat = match options["repeat"].as_str() {
            Some("off") => Repeat::Off,
            Some("context") => Repeat::Context,
            Some("track") => Repeat::Track,
            _ => self.repeat,
        };
    }
}

fn status(value: &Value) -> Option<Status> {
    match value.as_str()? {
        // Buffering is playback that has not caught up yet.
        "playing" | "buffering" => Some(Status::Playing),
        "paused" => Some(Status::Paused),
        "idle" => Some(Status::Stopped),
        _ => None,
    }
}

fn volume(value: &Value) -> Option<u8> {
    value.as_u64().map(|volume| volume.min(100) as u8)
}

fn anchor(value: &Value) -> Option<Anchor> {
    Some(Anchor {
        position_ms: value["position_ms"].as_i64()?,
        timestamp_ms: value["timestamp_ms"].as_i64()?,
        speed: value["speed"].as_f64().unwrap_or(1.0),
    })
}

/// Soloist lists only the actions available now; without the list, offer all.
fn actions(value: &Value) -> Actions {
    let Some(available) = value.as_object() else {
        return Actions {
            play: true,
            pause: true,
            next: true,
            previous: true,
            seek: true,
        };
    };
    Actions {
        play: available.contains_key("play"),
        pause: available.contains_key("pause"),
        next: available.contains_key("skip_next"),
        previous: available.contains_key("skip_prev"),
        seek: available.contains_key("seek"),
    }
}

fn name(entity: &Value) -> String {
    entity["decorations"]["identity"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn track(item: &Value) -> Option<Track> {
    let uri = item["uri"].as_str().filter(|uri| !uri.is_empty())?;
    let decorations = &item["decorations"];
    let artists = decorations["creators"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|creator| name(&creator["entity"]))
        .filter(|artist| !artist.is_empty())
        .collect();
    Some(Track {
        uri: uri.to_owned(),
        title: name(item),
        artists,
        album: name(&decorations["parent"]["entity"]),
        art_url: cover(&decorations["visual_identity"]["cover"]),
        duration_ms: decorations["playback"]["duration_ms"].as_i64(),
    })
}

/// The large cover when there is one: sharp in a bar, without the biggest download.
fn cover(covers: &Value) -> Option<String> {
    let covers = covers.as_array()?;
    let url = |size: &str| {
        covers
            .iter()
            .find(|cover| cover["size"] == size)
            .and_then(|cover| cover["url"].as_str())
    };
    ["large", "xlarge", "default", "small"]
        .into_iter()
        .find_map(url)
        .or_else(|| covers.first().and_then(|cover| cover["url"].as_str()))
        .map(str::to_owned)
}

/// The MPRIS track id (a D-Bus object path) for a Spotify URI.
pub fn track_path(uri: &str) -> String {
    if uri.is_empty() {
        return "/org/mpris/MediaPlayer2/TrackList/NoTrack".to_owned();
    }
    let element: String = uri
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("/org/mpris/MediaPlayer2/soloist/{element}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity() -> Value {
        json!({
            "uri": "spotify:track:2JRo0gjbX4GrCqBYdRohoo",
            "entity_type": "track",
            "decorations": {
                "identity": { "name": "My Song" },
                "visual_identity": { "cover": [
                    { "url": "https://i.scdn.co/image/small", "size": "small" },
                    { "url": "https://i.scdn.co/image/large", "size": "large" }
                ] },
                "parent": { "entity": {
                    "uri": "spotify:album:4aawyAB9vmqN3uQ7FjRGTy",
                    "entity_type": "album",
                    "decorations": { "identity": { "name": "Album Name" } }
                } },
                "creators": [
                    { "entity": { "entity_type": "artist", "decorations": { "identity": { "name": "First" } } } },
                    { "entity": { "entity_type": "artist", "decorations": { "identity": { "name": "Second" } } } }
                ],
                "playback": { "duration_ms": 210000, "content_ratings": [] }
            }
        })
    }

    fn snapshot() -> Value {
        json!({
            "type": "playback_state",
            "status": "playing",
            "item": entity(),
            "position": { "position_ms": 45000, "timestamp_ms": 1_000_000, "speed": 1.0 },
            "volume": 65,
            "is_active": true,
            "options": { "shuffle": true, "repeat": "context", "playback_speed": 1.0, "modes": {} },
            "available_actions": {
                "pause": {}, "seek": {}, "skip_next": {}, "skip_prev": {}
            }
        })
    }

    #[test]
    fn a_snapshot_fills_the_whole_state() {
        let mut state = State::default();
        let changes = state.apply(&snapshot());
        assert_eq!(
            changes,
            Changes {
                metadata: true,
                status: true,
                volume: true,
                options: true,
                actions: true,
                seeked: true
            }
        );
        assert_eq!(state.status, Status::Playing);
        assert_eq!(
            state.track,
            Some(Track {
                uri: "spotify:track:2JRo0gjbX4GrCqBYdRohoo".into(),
                title: "My Song".into(),
                artists: vec!["First".into(), "Second".into()],
                album: "Album Name".into(),
                art_url: Some("https://i.scdn.co/image/large".into()),
                duration_ms: Some(210000),
            })
        );
        assert_eq!(state.volume, 65);
        assert!(state.shuffle);
        assert_eq!(state.repeat, Repeat::Context);
        assert_eq!(
            state.actions,
            Actions {
                play: false,
                pause: true,
                next: true,
                previous: true,
                seek: true
            }
        );
    }

    #[test]
    fn granular_events_change_only_their_part() {
        let mut state = State::default();
        state.apply(&snapshot());

        let changes = state.apply(&json!({ "type": "playback_changed", "status": "paused" }));
        assert_eq!(
            changes,
            Changes {
                status: true,
                ..Changes::default()
            }
        );
        assert_eq!(state.status, Status::Paused);

        let changes = state.apply(&json!({ "type": "volume_changed", "volume": 42 }));
        assert_eq!(
            changes,
            Changes {
                volume: true,
                ..Changes::default()
            }
        );
        assert_eq!(state.volume, 42);

        let options = json!({ "type": "options_changed", "options": { "shuffle": false, "repeat": "track" } });
        assert_eq!(
            state.apply(&options),
            Changes {
                options: true,
                ..Changes::default()
            }
        );
        assert_eq!((state.shuffle, state.repeat), (false, Repeat::Track));

        let mut next = entity();
        next["uri"] = json!("spotify:track:other");
        next["decorations"]["identity"]["name"] = json!("Other Song");
        let changes = state.apply(&json!({ "type": "track_changed", "item": next }));
        assert_eq!(
            changes,
            Changes {
                metadata: true,
                ..Changes::default()
            }
        );
        assert_eq!(
            state.track.as_ref().map(|t| t.title.as_str()),
            Some("Other Song")
        );
    }

    #[test]
    fn idle_and_buffering_map_to_mpris_states() {
        let mut state = State::default();
        state.apply(&json!({ "type": "playback_changed", "status": "buffering" }));
        assert_eq!(state.status, Status::Playing);
        state.apply(&json!({ "type": "playback_changed", "status": "idle" }));
        assert_eq!(state.status, Status::Stopped);
    }

    #[test]
    fn an_empty_item_means_nothing_is_playing() {
        let mut state = State::default();
        state.apply(&snapshot());
        let changes = state
            .apply(&json!({ "type": "track_changed", "item": { "uri": "", "entity_type": "" } }));
        assert!(changes.metadata);
        assert_eq!(state.track, None);
    }

    #[test]
    fn position_moves_on_from_its_anchor_and_stops_at_the_end() {
        let mut state = State::default();
        state.apply(&snapshot());
        assert_eq!(state.position_ms(1_000_000), 45000);
        assert_eq!(state.position_ms(1_002_500), 47500);
        assert_eq!(
            state.position_ms(9_000_000),
            210000,
            "never past the track's length"
        );

        state.apply(&json!({ "type": "position_sync", "position": { "position_ms": 50000, "timestamp_ms": 1_005_000, "speed": 0.0 } }));
        assert_eq!(
            state.position_ms(1_100_000),
            50000,
            "speed 0 holds the position"
        );
    }

    #[test]
    fn a_position_sync_is_a_seek_only_when_it_jumps() {
        let mut state = State::default();
        state.apply(&snapshot());
        let on_time = json!({ "type": "position_sync", "position": { "position_ms": 55000, "timestamp_ms": 1_010_000, "speed": 1.0 } });
        assert_eq!(state.apply(&on_time), Changes::default());
        let jump = json!({ "type": "position_sync", "position": { "position_ms": 120000, "timestamp_ms": 1_011_000, "speed": 1.0 } });
        assert_eq!(
            state.apply(&jump),
            Changes {
                seeked: true,
                ..Changes::default()
            }
        );
    }

    #[test]
    fn logging_out_forgets_the_track() {
        let mut state = State::default();
        state.apply(&snapshot());
        let changes = state.apply(&json!({ "type": "auth_state", "logged_in": false, "is_active": false, "device_name": "ThinkPad" }));
        assert!(changes.metadata && changes.status);
        assert_eq!(
            (state.logged_in, state.track.clone(), state.status),
            (false, None, Status::Stopped)
        );
    }

    #[test]
    fn handing_playback_to_another_device_stops_it_here() {
        let mut state = State::default();
        state.apply(&snapshot());
        let away =
            json!({ "type": "device_changed", "is_active": false, "device_name": "ThinkPad" });
        assert_eq!(
            state.apply(&away),
            Changes {
                status: true,
                ..Changes::default()
            }
        );
        assert_eq!((state.inactive, state.status), (true, Status::Stopped));
        assert!(
            state.track.is_some(),
            "the last track stays on the player's tile"
        );

        let remote = json!({ "type": "playback_changed", "status": "playing" });
        assert_eq!(
            state.apply(&remote),
            Changes::default(),
            "another device's playback"
        );
        assert_eq!(state.status, Status::Stopped);

        let back =
            json!({ "type": "device_changed", "is_active": true, "device_name": "ThinkPad" });
        state.apply(&back);
        state.apply(&remote);
        assert_eq!((state.inactive, state.status), (false, Status::Playing));
    }

    #[test]
    fn a_snapshot_of_an_inactive_device_is_stopped() {
        let mut snapshot = snapshot();
        snapshot["is_active"] = json!(false);
        let mut state = State::default();
        state.apply(&snapshot);
        assert_eq!((state.inactive, state.status), (true, Status::Stopped));
    }

    #[test]
    fn unknown_and_reply_messages_change_nothing() {
        let mut state = State::default();
        state.apply(&snapshot());
        let before = state.clone();
        for event in [
            json!({ "type": "command_result", "command": "pause" }),
            json!({ "type": "error", "message": "unknown command" }),
            json!({ "type": "queue_changed", "previous": [], "upcoming": [] }),
            json!({ "something": "else" }),
        ] {
            assert_eq!(state.apply(&event), Changes::default());
        }
        assert_eq!(state, before);
    }

    #[test]
    fn track_paths_are_valid_object_paths() {
        assert_eq!(
            track_path("spotify:track:2JRo0gjbX4GrCqBYdRohoo"),
            "/org/mpris/MediaPlayer2/soloist/spotify_track_2JRo0gjbX4GrCqBYdRohoo"
        );
        assert_eq!(track_path(""), "/org/mpris/MediaPlayer2/TrackList/NoTrack");
    }
}
