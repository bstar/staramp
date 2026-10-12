//! Embedded output on the presentation machine. Bounded PCM, never a device tap.
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, OnceLock,
};

pub const FRAMES: usize = 960;
#[derive(Debug)]
pub struct Block {
    pub epoch: u64,
    pub samples: Vec<i16>,
    pub pcm: Option<(
        starkit::pcm::Spec,
        starkit::pcm::SampleFormat,
        bool,
        Vec<u8>,
    )>,
}
static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
static PIPE: OnceLock<(Sender<Block>, Receiver<Block>)> = OnceLock::new();
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Release);
}
pub fn receiver() -> Receiver<Block> {
    PIPE.get_or_init(|| bounded(8)).1.clone()
}
pub fn next_epoch() -> u64 {
    for _ in receiver().try_iter() {}
    reset_credit();
    PLAYED.store(0, Ordering::Release);
    EPOCH.fetch_add(1, Ordering::Relaxed)
}
pub fn send(block: Block, stop: &Arc<AtomicBool>) -> bool {
    let sender = &PIPE.get_or_init(|| bounded(8)).0;
    let mut block = block;
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        match sender.send_timeout(block, std::time::Duration::from_millis(10)) {
            Ok(()) => return true,
            Err(crossbeam_channel::SendTimeoutError::Timeout(value)) => block = value,
            Err(_) => return false,
        }
    }
}

static CREDIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn reset_credit() {
    CREDIT.store(0, Ordering::Relaxed);
}
pub fn credit(count: usize) {
    let _ = CREDIT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        Some(n.saturating_add(count).min(8))
    });
}
pub fn take_credit() -> bool {
    CREDIT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_ok()
}

pub fn current_epoch() -> u64 {
    EPOCH.load(Ordering::Relaxed).saturating_sub(1)
}

static PRECISE: AtomicBool = AtomicBool::new(false);
pub fn precise() -> bool {
    PRECISE.load(Ordering::Acquire)
}
pub fn set_precise(value: bool) {
    PRECISE.store(value, Ordering::Release);
}
static ROUTE: AtomicU64 = AtomicU64::new(0);
static COMPATIBILITY: AtomicBool = AtomicBool::new(false);
pub fn route() -> u64 {
    ROUTE.load(Ordering::Acquire)
}
pub fn claim(token: u64) {
    ROUTE.store(token, Ordering::Release);
}
pub fn compatibility() -> bool {
    COMPATIBILITY.load(Ordering::Acquire)
}
pub fn set_compatibility(value: bool) {
    COMPATIBILITY.store(value, Ordering::Release);
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Packet {
    pub epoch: u64,
    pub spec: starkit::pcm::Spec,
    pub data: String,
    pub format: starkit::pcm::SampleFormat,
    pub processed: bool,
}
pub fn packet() -> Option<Packet> {
    use base64::Engine as _;
    while let Ok(block) = receiver().try_recv() {
        if block.epoch != current_epoch() {
            continue;
        }
        if let Some((spec, format, processed, data)) = block.pcm {
            return Some(Packet {
                epoch: block.epoch,
                spec,
                format,
                processed,
                data: base64::engine::general_purpose::STANDARD.encode(data),
            });
        }
    }
    None
}

static DRAINING: AtomicBool = AtomicBool::new(false);
pub fn draining() -> bool {
    DRAINING.load(Ordering::Acquire)
}
pub fn set_draining(value: bool) {
    DRAINING.store(value, Ordering::Release);
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Poll {
    pub playing: bool,
    pub paused: bool,
    pub draining: bool,
    pub finished: bool,
    pub compatibility: bool,
    pub packet: Option<Packet>,
}

static PLAYED: AtomicU64 = AtomicU64::new(0);
pub fn played() -> u64 {
    PLAYED.load(Ordering::Acquire)
}
pub fn publish_played(frames: u64) {
    PLAYED.fetch_max(frames, Ordering::AcqRel);
}
