//! One remote presentation lease on the application's existing playback owner.
use super::*;
use crate::audio::relay::Packet;
use starkit::terminal_graphics::{
    media::{ToClient, ToHost},
    protocol::ServerMessage,
};

pub(super) struct RemoteAudio {
    token: u64,
    active: bool,
    cache: bool,
    paused: bool,
    supported: bool,
    epoch: u64,
    awaiting_worker: Option<u64>,
    failed: bool,
    source: Option<(u64, starkit::pcm::Spec)>,
    pending: Option<Packet>,
    credits: usize,
    effects: Vec<ServerMessage>,
    notices: Vec<String>,
    compatibility: bool,
    restart: bool,
    ended: bool,
}
impl Default for RemoteAudio {
    fn default() -> Self {
        Self {
            token: ((std::process::id() as u64) << 32)
                ^ std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
            active: false,
            cache: false,
            paused: false,
            supported: false,
            epoch: 0,
            awaiting_worker: None,
            failed: false,
            source: None,
            pending: None,
            credits: 0,
            effects: Vec::new(),
            notices: Vec::new(),
            compatibility: false,
            restart: false,
            ended: false,
        }
    }
}
impl RemoteAudio {
    pub(super) fn active(&self) -> bool {
        self.active
    }
    fn route(&self, app: &App) -> String {
        app.audio_request(&format!(
            "audio-route {}{}",
            self.token,
            if self.cache { " cache" } else { "" }
        ))
    }
    fn close(&mut self) {
        if self.source.take().is_some() {
            self.effects.push(ServerMessage::Media {
                message: ToClient::PcmClose {
                    session: self.token,
                    epoch: self.epoch,
                },
            });
        }
        self.paused = false;
        self.pending = None;
        self.credits = 0;
        self.ended = false;
    }
    pub(super) fn capabilities(
        &mut self,
        app: &App,
        cap: starkit::terminal_graphics::capabilities::Capabilities,
    ) {
        if cap.local_media {
            return;
        }
        self.close();
        if !cap.audio_pcm_v2 {
            self.supported = false;
            self.active = app.audio_request(&format!("audio-route {}", self.token)) == "ok";
            app.audio_request("pause");
            self.notices.push(
                "Update the local STAR/KIT frontend for precision-preserving SSH audio".into(),
            );
            return;
        }
        self.awaiting_worker = None;
        self.failed = false;
        self.cache = cap.audio_cache_v1;
        let reply = self.route(app);
        self.supported = true;
        self.active = reply == "ok";
        if !self.active {
            self.notices.push(reply);
        }
        self.compatibility = app.audio_request("audio-compatibility") == "true";
    }
    pub(super) fn detached(&mut self, app: &App) {
        if self.active {
            app.audio_request(&format!("audio-detach {}", self.token));
        }
        self.active = false;
        self.restart = false;
        self.close();
    }
    pub(super) fn receive(&mut self, app: &App, msg: ToHost) {
        match msg {
            ToHost::AudioCredit {
                session,
                epoch,
                blocks,
            } if self.active && session == self.token && epoch == self.epoch => {
                self.credits = (self.credits + blocks.min(8)).min(8);
            }
            ToHost::AudioError {
                session,
                epoch,
                message,
            } if self.active && session == self.token && epoch == self.epoch => {
                app.audio_request("pause");
                self.close();
                self.failed = true;
                self.restart = true;
                self.notices.push(message);
            }
            ToHost::PcmBuffered {
                session,
                epoch,
                frames,
            } if self.active && self.cache && session == self.token && epoch == self.epoch => {
                if let Some((worker, _)) = self.source {
                    app.audio_request(&format!(
                        "audio-buffered {} {} {}",
                        self.token, worker, frames
                    ));
                }
            }
            ToHost::PcmProgress {
                session,
                epoch,
                frames,
            } if self.active && session == self.token && epoch == self.epoch => {
                if let Some((worker, _)) = self.source {
                    app.audio_request(&format!(
                        "audio-progress {} {} {}",
                        self.token, worker, frames
                    ));
                }
            }
            ToHost::PcmFinished { session, epoch }
                if self.active && session == self.token && epoch == self.epoch =>
            {
                if let Some((worker, _)) = self.source {
                    app.audio_request(&format!("audio-finished {} {}", self.token, worker));
                }
                self.close();
            }
            ToHost::PcmReady {
                session,
                epoch,
                sample_rate,
                channels,
                format,
                converted,
            } if self.active && session == self.token && epoch == self.epoch => {
                let source = self
                    .source
                    .map(|(_, s)| {
                        format!(
                            "{} Hz / {}ch / {:?} {}-bit",
                            s.sample_rate,
                            s.channels,
                            s.format,
                            s.valid_bits.unwrap_or(
                                if s.format == starkit::pcm::SampleFormat::F64 {
                                    64
                                } else {
                                    32
                                }
                            )
                        )
                    })
                    .unwrap_or_default();
                self.notices.push(format!(
                    "SSH {source} -> local {sample_rate} Hz / {channels}ch / {format}{}",
                    if converted { " (converted)" } else { "" }
                ));
            }
            _ => {}
        }
    }
    pub(super) fn chunks(&mut self, app: &App) -> Vec<ServerMessage> {
        if !self.active || !self.supported {
            return Vec::new();
        }
        let paused = app.player.state.state() == PlayState::Paused;
        if self.failed && paused {
            return Vec::new();
        }
        if !paused {
            self.failed = false;
        }
        if app.player.state.state() == PlayState::Stopped || (paused && !self.cache) {
            if self.source.is_some() {
                self.close();
                self.restart = true;
            }
            return Vec::new();
        }
        let mut messages = Vec::new();
        if self.cache && self.source.is_some() && paused != self.paused {
            self.paused = paused;
            messages.push(ServerMessage::Media {
                message: ToClient::PcmPause {
                    session: self.token,
                    epoch: self.epoch,
                    paused,
                },
            });
            // Two packets at most fit the session media queue reservation.
            return messages;
        }
        // Opening a new epoch does not require credits. Chunks always do.
        for _ in 0..2 {
            if self.pending.is_none() {
                let reply = app.audio_request(&format!("audio-pcm {}", self.token));
                if reply.starts_with("error:") {
                    self.active = false;
                    self.close();
                    self.notices.push(reply);
                    break;
                }
                let Ok(poll) = serde_json::from_str::<crate::audio::relay::Poll>(&reply) else {
                    break;
                };
                if (poll.paused && !self.cache) || (!poll.playing && !poll.draining) {
                    self.close();
                    self.restart = true;
                    break;
                }
                if self.restart {
                    // Route changes are queued on the playback worker. Do not
                    // announce its old packets and then discard their credits
                    // when the replacement worker catches up.
                    self.awaiting_worker = Some(poll.epoch);
                    self.route(app);
                    self.restart = false;
                    break;
                }
                if let Some(old) = self.awaiting_worker {
                    if poll.packet.as_ref().is_some_and(|p| p.epoch != old) {
                        self.awaiting_worker = None;
                    } else {
                        break;
                    }
                }
                if poll.compatibility != self.compatibility {
                    self.close();
                    self.compatibility = poll.compatibility;
                }
                self.pending = poll.packet;
                if self.pending.is_none()
                    && poll.draining
                    && poll.finished
                    && self.source.is_some()
                    && !self.ended
                    && self.credits > 0
                {
                    messages.push(ServerMessage::Media {
                        message: ToClient::PcmEnd {
                            session: self.token,
                            epoch: self.epoch,
                        },
                    });
                    self.ended = true;
                }
            }
            let Some(packet) = self.pending.as_ref() else {
                break;
            };
            let key = (packet.epoch, packet.spec);
            if self.source != Some(key) {
                let spec = packet.spec;
                self.epoch = self.epoch.saturating_add(1);
                self.credits = 0;
                self.ended = false;
                self.paused = false;
                self.source = Some(key);
                messages.push(ServerMessage::Media {
                    message: ToClient::PcmOpen {
                        session: self.token,
                        epoch: self.epoch,
                        spec,
                        compatibility: self.compatibility,
                    },
                });
                if self.cache {
                    messages.push(ServerMessage::Media {
                        message: ToClient::PcmCache {
                            session: self.token,
                            epoch: self.epoch,
                        },
                    });
                }
                break;
            }
            if self.credits == 0 {
                break;
            }
            let packet = self.pending.take().unwrap();
            self.credits -= 1;
            messages.push(ServerMessage::Media {
                message: ToClient::PcmChunk {
                    session: self.token,
                    epoch: self.epoch,
                    format: packet.format,
                    processed: packet.processed,
                    data: packet.data,
                },
            });
        }
        messages
    }
    pub(super) fn effects(&mut self, app: &mut App) -> Vec<ServerMessage> {
        for notice in self.notices.drain(..) {
            app.audio_output = Some(notice.clone());
            app.note(notice);
        }
        std::mem::take(&mut self.effects)
    }
}
