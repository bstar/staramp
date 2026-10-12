//! MPRIS, so the desktop can see and control the player.
//!
//! The identity is `staramp`, which is what a status bar matches on. Note that
//! consumers frequently hardcode a list of player names — the reference
//! system's quickshell bar keys off `cliamp` and `mpd` explicitly — so a new
//! player is visible but not preferred until that list is updated.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use mpris_server::zbus::fdo;
use mpris_server::{
    LoopStatus, Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Signal,
    Time, TrackId, Volume,
};

use crate::audio::player::{Command, PlayState, Player};

pub const BUS_NAME: &str = "staramp";

#[derive(PartialEq)]
struct MprisSnapshot {
    volume: Option<f64>,
    shuffle: Option<bool>,
    loop_status: Option<LoopStatus>,
    next: Option<bool>,
    previous: Option<bool>,
    play: Option<bool>,
    pause: Option<bool>,
    seek: Option<bool>,
}

/// Bridges the player to the MPRIS interfaces.
pub struct MprisBridge {
    player: Arc<Player>,
    activity: crate::activity::Control,
    artwork: Arc<Mutex<Option<(String, String)>>>,
}

impl MprisBridge {
    pub fn new(
        player: Arc<Player>,
        activity: crate::activity::Control,
        artwork: Arc<Mutex<Option<(String, String)>>>,
    ) -> Self {
        Self {
            player,
            activity,
            artwork,
        }
    }

    fn track_id(&self) -> Option<TrackId> {
        self.player.current_item()?;
        let revision = self.player.state.track_revision.load(Ordering::Relaxed);
        TrackId::try_from(format!("/org/staramp/track/{revision}")).ok()
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
        Ok(Vec::new())
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
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
        if self.player.state.state() == PlayState::Playing {
            self.player.send(Command::Pause);
        }
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        match self.player.state.state() {
            PlayState::Playing => self.pause().await,
            PlayState::Paused | PlayState::Stopped => self.play().await,
        }
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.activity.manual_end();
        self.player.send(Command::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        if self.player.state.state() == PlayState::Stopped {
            if let Some(index) = self.player.queue.lock().unwrap().current_index() {
                self.player.send(Command::PlayIndex(index));
            }
        } else {
            self.player.send(Command::Resume);
        }
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        if !self.can_seek().await? {
            return Ok(());
        }
        self.activity.seek_end();
        self.player
            .send(Command::SeekBy(offset.as_micros() as f64 / 1_000_000.0));
        Ok(())
    }

    async fn set_position(&self, track: TrackId, position: Time) -> fdo::Result<()> {
        if self.track_id().as_ref() != Some(&track) || !self.can_seek().await? {
            return Ok(());
        }
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

    async fn set_loop_status(&self, status: LoopStatus) -> mpris_server::zbus::Result<()> {
        use crate::playlist::queue::RepeatMode;
        let mode = match status {
            LoopStatus::None => RepeatMode::Off,
            LoopStatus::Playlist => RepeatMode::All,
            LoopStatus::Track => RepeatMode::One,
        };
        self.player.queue.lock().unwrap().set_repeat(mode);
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
            m.set_album(item.album.clone());
            m.set_album_artist(item.album_artist.clone().map(|a| vec![a]));
            if let Some(d) = item.duration_secs {
                m.set_length(Some(Time::from_secs(d)));
            }
            if let Some(id) = self.track_id() {
                m.set_trackid(Some(id));
            }
            if let Ok(art) = self.artwork.lock() {
                if let Some((uri, url)) = art.as_ref() {
                    if uri == &item.uri.to_string() {
                        m.set_art_url(Some(url.clone()));
                    }
                }
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
        Ok(self.player.current_item().is_some() && self.player.queue.lock().unwrap().len() > 1)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.player.current_item().is_some() && self.player.queue.lock().unwrap().len() > 1)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(self.player.current_item().is_some())
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(self.player.state.state() == PlayState::Playing)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(self.player.current_item().is_some() && self.player.state.duration_secs() > 0.0)
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
    let artwork = Arc::new(Mutex::new(None));
    let bridge_artwork = Arc::clone(&artwork);

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
                let server =
                    match Server::new(BUS_NAME, MprisBridge::new(player, activity, bridge_artwork))
                        .await
                    {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("mpris unavailable: {e}");
                            return;
                        }
                    };
                tracing::info!("mpris: org.mpris.MediaPlayer2.{BUS_NAME}");

                let mut last_extra: Option<MprisSnapshot> = None;

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
                            let now = MprisSnapshot {
                                volume: server.imp().volume().await.ok(),
                                shuffle: server.imp().shuffle().await.ok(),
                                loop_status: server.imp().loop_status().await.ok(),
                                next: server.imp().can_go_next().await.ok(),
                                previous: server.imp().can_go_previous().await.ok(),
                                play: server.imp().can_play().await.ok(),
                                pause: server.imp().can_pause().await.ok(),
                                seek: server.imp().can_seek().await.ok(),
                            };
                            if last_extra.as_ref() != Some(&now) {
                                let mut changed = Vec::new();
                                if last_extra
                                    .as_ref()
                                    .is_none_or(|old| old.volume != now.volume)
                                {
                                    if let Some(v) = now.volume {
                                        changed.push(Property::Volume(v));
                                    }
                                }
                                if last_extra
                                    .as_ref()
                                    .is_none_or(|old| old.shuffle != now.shuffle)
                                {
                                    if let Some(v) = now.shuffle {
                                        changed.push(Property::Shuffle(v));
                                    }
                                }
                                if last_extra
                                    .as_ref()
                                    .is_none_or(|old| old.loop_status != now.loop_status)
                                {
                                    if let Some(v) = now.loop_status {
                                        changed.push(Property::LoopStatus(v));
                                    }
                                }
                                if last_extra.as_ref().is_none_or(|old| old.next != now.next) {
                                    if let Some(v) = now.next {
                                        changed.push(Property::CanGoNext(v));
                                    }
                                }
                                if last_extra
                                    .as_ref()
                                    .is_none_or(|old| old.previous != now.previous)
                                {
                                    if let Some(v) = now.previous {
                                        changed.push(Property::CanGoPrevious(v));
                                    }
                                }
                                if last_extra.as_ref().is_none_or(|old| old.play != now.play) {
                                    if let Some(v) = now.play {
                                        changed.push(Property::CanPlay(v));
                                    }
                                }
                                if last_extra.as_ref().is_none_or(|old| old.pause != now.pause) {
                                    if let Some(v) = now.pause {
                                        changed.push(Property::CanPause(v));
                                    }
                                }
                                if last_extra.as_ref().is_none_or(|old| old.seek != now.seek) {
                                    if let Some(v) = now.seek {
                                        changed.push(Property::CanSeek(v));
                                    }
                                }
                                if !changed.is_empty() {
                                    let _ = server.properties_changed(changed).await;
                                }
                                last_extra = Some(now);
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            });
        })
        .ok()?;

    Some(MprisHandle { tx, artwork })
}

pub enum MprisEvent {
    TrackChanged,
    StateChanged,
    Seeked(f64),
    Quit,
}

pub struct MprisHandle {
    tx: std::sync::mpsc::Sender<MprisEvent>,
    artwork: Arc<Mutex<Option<(String, String)>>>,
}

impl MprisHandle {
    pub fn notify(&self, e: MprisEvent) {
        let _ = self.tx.send(e);
    }

    pub fn set_artwork(&self, uri: Option<String>, url: Option<String>) {
        if let Ok(mut art) = self.artwork.lock() {
            *art = uri.zip(url);
        }
        self.notify(MprisEvent::TrackChanged);
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
