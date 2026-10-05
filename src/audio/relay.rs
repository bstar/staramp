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
