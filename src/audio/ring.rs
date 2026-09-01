//! The PCM ring between the decode thread and the output callback.
//!
//! Sized in **milliseconds, not frames**. 8192 frames sounds generous until you
//! play a hi-res file: it is 186 ms at 44.1 kHz but only 43 ms at 192 kHz, so a
//! fixed frame count silently loses its safety margin on exactly the files most
//! likely to be large and slow to read off a spinning USB disk.

use rtrb::{Consumer, Producer, RingBuffer};

/// How much audio the ring holds. Eight 1024-frame quanta of slack at 44.1 kHz,
/// enough to ride out a decode stall without the callback running dry, while
/// staying short enough that a seek is not audibly late.
pub const RING_MS: u32 = 200;

/// Ring capacity in samples for a given stream, rounded up to a power of two.
pub fn capacity_samples(sample_rate: u32, channels: u16) -> usize {
    let frames = (sample_rate as u64 * RING_MS as u64 / 1000).max(1024);
    (frames as usize * channels as usize).next_power_of_two()
}

pub fn create(sample_rate: u32, channels: u16) -> (Producer<f32>, Consumer<f32>) {
    RingBuffer::new(capacity_samples(sample_rate, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_scales_with_rate_not_a_fixed_frame_count() {
        let a = capacity_samples(44_100, 2);
        let b = capacity_samples(192_000, 2);
        assert!(b > a, "192kHz ring must be larger than 44.1kHz");
    }

    #[test]
    fn holds_at_least_the_target_duration() {
        for rate in [44_100, 48_000, 96_000, 192_000] {
            let samples = capacity_samples(rate, 2);
            let ms = (samples / 2) as u64 * 1000 / rate as u64;
            assert!(
                ms >= RING_MS as u64,
                "{rate}Hz ring holds {ms}ms, want >= {RING_MS}ms"
            );
        }
    }

    #[test]
    fn capacity_is_a_power_of_two() {
        for rate in [44_100, 48_000, 96_000, 192_000] {
            assert!(capacity_samples(rate, 2).is_power_of_two());
        }
    }
}
