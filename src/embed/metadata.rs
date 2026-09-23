//! Best-effort tags for the one track the embedded player is showing.
//!
//! A mounted file can block in the operating system. Keep that read off the
//! protocol/render thread, and keep at most one pending request behind it.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

use crate::library::tags;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataResult {
    pub path: String,
    pub revision: u64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

type Request = (String, u64);

pub struct MetadataWorker {
    requests: Sender<Request>,
    /// A second receiver lets `request` replace an old queued track while
    /// the worker is still reading a slow mount.
    pending: Receiver<Request>,
    latest: Arc<Mutex<Option<MetadataResult>>>,
}

impl MetadataWorker {
    pub fn new() -> Self {
        let (requests, pending) = bounded::<Request>(1);
        let latest = Arc::new(Mutex::new(None));
        let worker_pending = pending.clone();
        let worker_latest = Arc::clone(&latest);
        thread::Builder::new()
            .name("staramp-embed-tags".into())
            .spawn(move || {
                while let Ok((path, revision)) = worker_pending.recv() {
                    let result = match tags::read(Path::new(&path)) {
                        Ok(tags) => MetadataResult {
                            path,
                            revision,
                            title: tags.title,
                            artist: tags.artist,
                            album: tags.album,
                        },
                        Err(_) => MetadataResult {
                            path,
                            revision,
                            title: None,
                            artist: None,
                            album: None,
                        },
                    };
                    *worker_latest
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner()) = Some(result);
                }
            })
            .expect("metadata worker thread can start");
        Self {
            requests,
            pending,
            latest,
        }
    }

    pub fn request(&self, path: String, revision: u64) {
        let request = (path, revision);
        match self.requests.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(request)) => {
                // Only the current track matters. If the worker picked up
                // the old request between these calls, the send now fits.
                let _ = self.pending.try_recv();
                let _ = self.requests.try_send(request);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    pub fn try_latest(&self) -> Option<MetadataResult> {
        self.latest
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }
}

impl Default for MetadataWorker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn newest_request_eventually_wins_without_waiting_for_old_tags() {
        let worker = MetadataWorker::new();
        for revision in 0..20 {
            worker.request(format!("/no-such-track-{revision}.flac"), revision);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = worker.try_latest() {
                if result.revision == 19 {
                    assert_eq!(result.path, "/no-such-track-19.flac");
                    assert_eq!(result.title, None);
                    break;
                }
            }
            assert!(Instant::now() < deadline, "the newest tags did not arrive");
            thread::sleep(Duration::from_millis(5));
        }
    }
}
