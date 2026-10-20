//! soloist-mpris: Spotify Soloist as an MPRIS player.
//!
//! Soloist plays headless and speaks only its local WebSocket API. This
//! bridge follows that socket and publishes `org.mpris.MediaPlayer2.soloist`
//! on the session bus, so bars, media keys and `playerctl` see and control
//! it. Soloist must run with `--ws 127.0.0.1:0`; the bridge finds the port in
//! Soloist's data directory and reconnects whenever Soloist restarts.

mod cli;
mod command;
mod resume;
mod state;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use mpris_server::zbus::{self, fdo};
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property, RootInterface,
    Server, Signal, Time, TrackId, Volume,
};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use command::Request;
use state::{Changes, Repeat, State, Status};

const RETRY: Duration = Duration::from_secs(2);
/// The well-known name the bridge publishes the player under.
const BUS_NAME: &str = "org.mpris.MediaPlayer2.soloist";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The MPRIS side: answers D-Bus reads from the shared state and turns
/// D-Bus calls into Soloist commands for the socket task.
struct Bridge {
    state: Arc<Mutex<State>>,
    commands: mpsc::UnboundedSender<Value>,
}

impl Bridge {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn send(&self, request: Request) {
        let commands = command::commands(request, &self.state(), now_ms());
        for command in commands {
            // A closed channel means the bridge is shutting down.
            let _ = self.commands.send(command);
        }
    }
}

fn playback_status(state: &State) -> PlaybackStatus {
    match state.status {
        Status::Playing => PlaybackStatus::Playing,
        Status::Paused => PlaybackStatus::Paused,
        Status::Stopped => PlaybackStatus::Stopped,
    }
}

fn loop_status(state: &State) -> LoopStatus {
    match state.repeat {
        Repeat::Off => LoopStatus::None,
        Repeat::Context => LoopStatus::Playlist,
        Repeat::Track => LoopStatus::Track,
    }
}

fn metadata(state: &State) -> Metadata {
    let Some(track) = &state.track else {
        return Metadata::builder().trackid(TrackId::NO_TRACK).build();
    };
    let trackid = TrackId::try_from(state::track_path(&track.uri)).unwrap_or(TrackId::NO_TRACK);
    let mut builder = Metadata::builder()
        .trackid(trackid)
        .title(track.title.clone())
        .artist(track.artists.clone())
        .album(track.album.clone())
        .url(format!(
            "https://open.spotify.com/{}",
            track.uri.trim_start_matches("spotify:").replace(':', "/")
        ));
    if let Some(art_url) = &track.art_url {
        builder = builder.art_url(art_url.clone());
    }
    if let Some(duration_ms) = track.duration_ms {
        builder = builder.length(Time::from_millis(duration_ms));
    }
    builder.build()
}

impl RootInterface for Bridge {
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("Spotify Soloist".to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        // Players use their desktop entry for an icon; Spotifast is the app
        // that drives this speaker.
        Ok("spotifast".to_owned())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl PlayerInterface for Bridge {
    async fn next(&self) -> fdo::Result<()> {
        self.send(Request::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.send(Request::Previous);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.send(Request::Pause);
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        self.send(Request::PlayPause);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.send(Request::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.send(Request::Play);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.send(Request::SeekBy(offset.as_millis()));
        Ok(())
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        // MPRIS: a position for a track that is no longer current is ignored.
        let current = self
            .state()
            .track
            .as_ref()
            .map(|track| state::track_path(&track.uri));
        if current.as_deref() == Some(track_id.as_str()) {
            self.send(Request::SeekTo(position.as_millis()));
        }
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "open a song from Spotifast".to_owned(),
        ))
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(playback_status(&self.state()))
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(loop_status(&self.state()))
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> zbus::Result<()> {
        self.send(Request::Repeat(match loop_status {
            LoopStatus::None => Repeat::Off,
            LoopStatus::Playlist => Repeat::Context,
            LoopStatus::Track => Repeat::Track,
        }));
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: PlaybackRate) -> zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.state().shuffle)
    }

    async fn set_shuffle(&self, shuffle: bool) -> zbus::Result<()> {
        self.send(Request::Shuffle(shuffle));
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(metadata(&self.state()))
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(f64::from(self.state().volume) / 100.0)
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        self.send(Request::Volume(volume));
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_millis(self.state().position_ms(now_ms())))
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self.state().actions.next)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.state().actions.previous)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        let state = self.state();
        Ok(state.logged_in && state.track.is_some())
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(self.state().actions.pause)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.state().actions.seek)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// The properties and signal that announce `changes`.
async fn announce(server: &Server<Bridge>, changes: Changes) -> zbus::Result<()> {
    let state = server.imp().state().clone();
    let mut properties = Vec::new();
    if changes.metadata {
        properties.push(Property::Metadata(metadata(&state)));
        properties.push(Property::CanPlay(state.logged_in && state.track.is_some()));
    }
    if changes.status {
        properties.push(Property::PlaybackStatus(playback_status(&state)));
    }
    if changes.volume {
        properties.push(Property::Volume(f64::from(state.volume) / 100.0));
    }
    if changes.options {
        properties.push(Property::Shuffle(state.shuffle));
        properties.push(Property::LoopStatus(loop_status(&state)));
    }
    if changes.actions {
        properties.push(Property::CanGoNext(state.actions.next));
        properties.push(Property::CanGoPrevious(state.actions.previous));
        properties.push(Property::CanPause(state.actions.pause));
        properties.push(Property::CanSeek(state.actions.seek));
    }
    if !properties.is_empty() {
        server.properties_changed(properties).await?;
    }
    if changes.seeked {
        server
            .emit(Signal::Seeked {
                position: Time::from_millis(state.position_ms(now_ms())),
            })
            .await?;
    }
    Ok(())
}

fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SOLOIST_DATA_DIR") {
        return dir.into();
    }
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    data_home.join("soloist")
}

/// Soloist's WebSocket address, from the files it writes while running.
async fn address(dir: &std::path::Path) -> Option<String> {
    let addr = tokio::fs::read_to_string(dir.join("ws.addr")).await.ok()?;
    let port = tokio::fs::read_to_string(dir.join("ws.port")).await.ok()?;
    Some(format!("ws://{}:{}", addr.trim(), port.trim()))
}

/// Where the bridge keeps what this speaker was doing when it lost Soloist.
struct Memory {
    path: PathBuf,
    boot_id: String,
}

impl Memory {
    fn new() -> Self {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
            });
        let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map(|id| id.trim().to_owned())
            .unwrap_or_default();
        Memory {
            path: state_home.join("soloist-mpris/resume.json"),
            boot_id,
        }
    }

    /// Saves `state`, unless Soloist never said who has playback.
    async fn save(&self, state: &State) {
        let Some(saved) = resume::Saved::of(state, &self.boot_id, now_ms()) else {
            return;
        };
        let Ok(json) = serde_json::to_vec(&saved) else {
            return;
        };
        // Written aside and renamed, so a crash never leaves half a file. A
        // SIGTERM can land while a save is under way and start another, so
        // every save writes its own partial file.
        static SAVES: AtomicU64 = AtomicU64::new(0);
        let save = SAVES.fetch_add(1, Ordering::Relaxed);
        let partial = self
            .path
            .with_extension(format!("json.{}.{save}.partial", std::process::id()));
        let written = async {
            if let Some(parent) = self.path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&partial, json).await?;
            tokio::fs::rename(&partial, &self.path).await
        };
        if let Err(error) = written.await {
            eprintln!("saving {}: {error}", self.path.display());
        }
    }

    async fn load(&self) -> Option<resume::Saved> {
        let json = tokio::fs::read(&self.path).await.ok()?;
        serde_json::from_slice(&json).ok()
    }
}

/// Follows one connection until it closes.
async fn follow(
    server: &Server<Bridge>,
    url: &str,
    commands: &mut mpsc::UnboundedReceiver<Value>,
    memory: &Memory,
) {
    let Ok((socket, _)) = tokio_tungstenite::connect_async(url).await else {
        return;
    };
    eprintln!("connected to {url}");
    let (mut sink, mut stream) = socket.split();
    // Decided once per connection, on its first playback snapshot.
    let mut resume_checked = false;
    loop {
        tokio::select! {
            message = stream.next() => {
                let Some(Ok(message)) = message else { break };
                let Message::Text(text) = message else { continue };
                let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
                if event["type"] == "error" {
                    eprintln!("soloist: {}", event["message"]);
                }
                let changes = server.imp().state().apply_at(&event, now_ms());
                if changes.refresh {
                    let get_state = serde_json::json!({ "type": "command", "command": "get_state" });
                    if sink.send(Message::Text(get_state.to_string().into())).await.is_err() {
                        break;
                    }
                }
                if let Err(error) = announce(server, changes).await {
                    eprintln!("D-Bus: {error}");
                }
                if !resume_checked && event["type"] == "playback_state" {
                    resume_checked = true;
                    let saved = memory.load().await;
                    let state = server.imp().state().clone();
                    let take_back = resume::commands(saved.as_ref(), &state, &memory.boot_id, now_ms());
                    if !take_back.is_empty() {
                        eprintln!("Soloist restarted while it had playback; taking it back");
                    }
                    for command in take_back {
                        if sink.send(Message::Text(command.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            Some(command) = commands.recv() => {
                if sink.send(Message::Text(command.to_string().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    eprintln!("disconnected from {url}");
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> zbus::Result<()> {
    match cli::parse(std::env::args().skip(1)) {
        cli::Cli::Run => {}
        cli::Cli::Help => {
            println!("{}", cli::USAGE);
            return Ok(());
        }
        cli::Cli::Version => {
            println!("soloist-mpris {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        cli::Cli::Unknown(arg) => {
            eprintln!("soloist-mpris: unknown argument {arg}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    }
    // A second bridge would take the player's name from the running one and
    // leave the desktop without a player when it exits.
    let session = zbus::Connection::session().await?;
    let name = zbus::names::BusName::try_from(BUS_NAME)?;
    if zbus::fdo::DBusProxy::new(&session)
        .await?
        .name_has_owner(name)
        .await?
    {
        eprintln!("soloist-mpris: {BUS_NAME} already has a bridge; not starting another");
        std::process::exit(1);
    }
    drop(session);

    let (sender, mut commands) = mpsc::unbounded_channel();
    let state = Arc::new(Mutex::new(State::default()));
    let server = Server::new(
        "soloist",
        Bridge {
            state,
            commands: sender,
        },
    )
    .await?;
    let memory = Memory::new();
    // Stopping the bridge (Soloist's restart stops it too) saves what this
    // speaker was doing, so the next start can take playback back.
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = bridge(&server, &mut commands, &memory) => result,
        _ = terminate.recv() => {
            let state = server.imp().state().clone();
            memory.save(&state).await;
            Ok(())
        }
    }
}

async fn bridge(
    server: &Server<Bridge>,
    commands: &mut mpsc::UnboundedReceiver<Value>,
    memory: &Memory,
) -> zbus::Result<()> {
    let dir = data_dir();
    loop {
        if let Some(url) = address(&dir).await {
            follow(server, &url, commands, memory).await;
            let state = server.imp().state().clone();
            memory.save(&state).await;
            // Nothing is known while Soloist is away.
            *server.imp().state() = State::default();
            announce(
                server,
                Changes {
                    metadata: true,
                    status: true,
                    actions: true,
                    ..Changes::default()
                },
            )
            .await?;
            // Commands sent while disconnected are stale by the next connection.
            while commands.try_recv().is_ok() {}
        }
        tokio::time::sleep(RETRY).await;
    }
}
