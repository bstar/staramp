//! MPRIS, so the desktop can see and control the player.
//!
//! The identity is `staramp`, which is what a status bar matches on. Note that
//! consumers frequently hardcode a list of player names — the reference
//! system's quickshell bar keys off `cliamp` and `mpd` explicitly — so a new
//! player is visible but not preferred until that list is updated.

use std::sync::Arc;

use mpris_server::zbus::fdo;
use mpris_server::{
    LoopStatus, Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Signal,
    Time, TrackId, Volume,
};

use crate::audio::player::{Command, PlayState, Player};

pub const BUS_NAME: &str = "staramp";

/// Bridges the player to the MPRIS interfaces.
pub struct MprisBridge {
    player: Arc<Player>,
    activity: crate::activity::Control,
}

impl MprisBridge {
    pub fn new(player: Arc<Player>, activity: crate::activity::Control) -> Self {
        Self { player, activity }
    }
}

impl RootInterface for MprisBridge {
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

    async fn set_fullscreen(&self, _: bool) -> mpris_server::zbus::Result<()> {
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
        Ok("staramp".into())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("staramp".into())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["file".into()])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![
            "audio/flac".into(),
            "audio/mpeg".into(),
            "audio/ogg".into(),
            "audio/mp4".into(),
            "audio/x-ape".into(),
            "audio/x-wavpack".into(),
            "audio/x-musepack".into(),
        ])
    }
}

impl PlayerInterface for MprisBridge {
    async fn next(&self) -> fdo::Result<()> {
        self.activity.manual_end();
        self.player.send(Command::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.activity.manual_end();
        self.player.send(Command::Prev);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.player.send(Command::Pause);
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        self.player.send(Command::TogglePause);
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.activity.manual_end();
        self.player.send(Command::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.player.send(Command::Resume);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.activity.seek_end();
        self.player
            .send(Command::SeekBy(offset.as_micros() as f64 / 1_000_000.0));
        Ok(())
    }

    async fn set_position(&self, _track: TrackId, position: Time) -> fdo::Result<()> {
        self.activity.seek_end();
        self.player
            .send(Command::SeekTo(position.as_micros() as f64 / 1_000_000.0));
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Ok(())
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(match self.player.state.state() {
            PlayState::Playing => PlaybackStatus::Playing,
            PlayState::Paused => PlaybackStatus::Paused,
            PlayState::Stopped => PlaybackStatus::Stopped,
        })
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        use crate::playlist::queue::RepeatMode;
        Ok(match self.player.queue.lock().unwrap().repeat() {
            RepeatMode::Off => LoopStatus::None,
            RepeatMode::All => LoopStatus::Playlist,
            RepeatMode::One => LoopStatus::Track,
        })
    }

    async fn set_loop_status(&self, _: LoopStatus) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn set_rate(&self, _: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.player.queue.lock().unwrap().shuffled())
    }

    async fn set_shuffle(&self, on: bool) -> mpris_server::zbus::Result<()> {
        let mut q = self.player.queue.lock().unwrap();
        if q.shuffled() != on {
            q.toggle_shuffle();
        }
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        let mut m = Metadata::new();
        if let Some(item) = self.player.current_item() {
            if let Some(t) = &item.title {
                m.set_title(Some(t.clone()));
            }
            if let Some(a) = &item.artist {
                m.set_artist(Some(vec![a.clone()]));
            }
            if let Some(d) = item.duration_secs {
                m.set_length(Some(Time::from_secs(d)));
            }
            // A stable id per queue position; MPRIS requires a valid object path.
            let idx = self
                .player
                .queue
                .lock()
                .unwrap()
                .current_index()
                .unwrap_or(0);
            if let Ok(id) = TrackId::try_from(format!("/org/staramp/track/{idx}")) {
                m.set_trackid(Some(id));
            }
        }
        Ok(m)
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.player.volume() as f64)
    }

    async fn set_volume(&self, v: Volume) -> mpris_server::zbus::Result<()> {
        self.player.set_volume(v as f32);
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(Time::from_micros(
            (self.player.state.position_secs() * 1_000_000.0) as i64,
        ))
    }

    async fn minimum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<f64> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

/// Start the MPRIS server on its own thread with its own runtime.
///
/// Returns a handle that pushes property changes; dropping it stops the server.
/// Failure is not fatal: a missing session bus means no desktop integration,
/// not a player that refuses to start.
pub fn spawn(player: Arc<Player>, activity: crate::activity::Control) -> Option<MprisHandle> {
    let (tx, rx) = std::sync::mpsc::channel::<MprisEvent>();

    std::thread::Builder::new()
        .name("staramp-mpris".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!("mpris: no runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                let server = match Server::new(BUS_NAME, MprisBridge::new(player, activity)).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("mpris unavailable: {e}");
                        return;
                    }
                };
                tracing::info!("mpris: org.mpris.MediaPlayer2.{BUS_NAME}");

                loop {
                    // A blocking recv would park the runtime, so poll instead.
                    match rx.try_recv() {
                        Ok(MprisEvent::Quit) => break,
                        Ok(MprisEvent::TrackChanged) => {
                            let _ = server
                                .properties_changed([Property::Metadata(
                                    server.imp().metadata().await.unwrap_or_default(),
                                )])
                                .await;
                        }
                        Ok(MprisEvent::StateChanged) => {
                            if let Ok(st) = server.imp().playback_status().await {
                                let _ = server
                                    .properties_changed([Property::PlaybackStatus(st)])
                                    .await;
                            }
                        }
                        Ok(MprisEvent::Seeked(secs)) => {
                            let _ = server
                                .emit(Signal::Seeked {
                                    position: Time::from_micros((secs * 1_000_000.0) as i64),
                                })
                                .await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            });
        })
        .ok()?;

    Some(MprisHandle { tx })
}

pub enum MprisEvent {
    TrackChanged,
    StateChanged,
    Seeked(f64),
    Quit,
}

pub struct MprisHandle {
    tx: std::sync::mpsc::Sender<MprisEvent>,
}

impl MprisHandle {
    pub fn notify(&self, e: MprisEvent) {
        let _ = self.tx.send(e);
    }
}

impl Drop for MprisHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(MprisEvent::Quit);
    }
}

/// Check for a session bus without starting anything.
pub fn session_bus_available() -> bool {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
        || std::path::Path::new(&format!("/run/user/{}/bus", unsafe { libc_getuid() })).exists()
}

extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_bus_name_is_what_status_bars_match_on() {
        assert_eq!(super::BUS_NAME, "staramp");
    }
}
