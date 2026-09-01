//! Text effects.
//!
//! The effect designs follow TerminalTextEffects and its Rust port ttfx
//! (github.com/omacom/ttfx, MIT), which this system already uses for its
//! screensaver and branding. They are reimplemented here rather than depended
//! on because those render ANSI to stdout, while staramp needs per-character
//! state it can paint into a ratatui buffer with its own theme colours — and
//! because the interesting part is driving them from the audio, which a
//! stdout-oriented engine cannot do.
//!
//! Restraint is deliberate. An effect fires on every track change, so it has to
//! be short, skippable, and completely absent when asked for.

use crate::util::rng::Lcg;

pub mod reactive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// Characters resolve out of random glyphs, one by one.
    Decrypt,
    /// Text slides in from the right.
    Slide,
    /// Revealed left to right.
    Wipe,
    /// Characters appear in random order.
    Scatter,
    /// Nothing at all.
    None,
}

impl EffectKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "decrypt" => EffectKind::Decrypt,
            "slide" => EffectKind::Slide,
            "wipe" => EffectKind::Wipe,
            "scatter" => EffectKind::Scatter,
            "none" | "off" => EffectKind::None,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Decrypt => "decrypt",
            EffectKind::Slide => "slide",
            EffectKind::Wipe => "wipe",
            EffectKind::Scatter => "scatter",
            EffectKind::None => "none",
        }
    }

    pub fn all() -> &'static [EffectKind] {
        &[
            EffectKind::Decrypt,
            EffectKind::Slide,
            EffectKind::Wipe,
            EffectKind::Scatter,
            EffectKind::None,
        ]
    }
}

/// Glyphs a character cycles through before it resolves.
const SCRAMBLE: &[char] = &[
    '#', '%', '&', '@', '*', '+', '=', '~', '/', '\\', '|', '<', '>', '?', '$',
];

/// A running text transition.
pub struct TextEffect {
    kind: EffectKind,
    target: Vec<char>,
    /// 0.0 to 1.0.
    progress: f32,
    /// Seconds the whole transition takes.
    duration: f32,
    /// Per-character reveal threshold, so characters do not all resolve at once.
    order: Vec<f32>,
    rng: Lcg,
    frame: u64,
}

impl TextEffect {
    pub fn new(kind: EffectKind, text: &str, duration: f32, seed: u64) -> Self {
        let target: Vec<char> = text.chars().collect();
        let mut rng = Lcg::new(seed);

        // Each character gets the fraction of the transition at which it
        // resolves. Ordering is what distinguishes the effects from each other.
        let n = target.len().max(1);
        let order: Vec<f32> = match kind {
            EffectKind::Wipe | EffectKind::Slide => (0..n).map(|i| i as f32 / n as f32).collect(),
            EffectKind::Decrypt => (0..n)
                .map(|i| {
                    // Mostly left to right, with enough jitter that it does not
                    // look like a wipe.
                    let base = i as f32 / n as f32;
                    (base * 0.7 + rng.next_f32() * 0.3).min(0.999)
                })
                .collect(),
            EffectKind::Scatter => (0..n).map(|_| rng.next_f32()).collect(),
            EffectKind::None => vec![0.0; n],
        };

        Self {
            kind,
            target,
            progress: if kind == EffectKind::None { 1.0 } else { 0.0 },
            duration: duration.max(0.01),
            order,
            rng,
            frame: 0,
        }
    }

    pub fn advance(&mut self, dt: f32) {
        self.progress = (self.progress + dt / self.duration).min(1.0);
        self.frame = self.frame.wrapping_add(1);
    }

    pub fn finished(&self) -> bool {
        self.progress >= 1.0
    }

    /// Skip to the end. Any keypress should do this — an animation that blocks
    /// the user is a worse animation.
    pub fn finish(&mut self) {
        self.progress = 1.0;
    }

    /// The text as it currently looks.
    pub fn render(&mut self) -> String {
        if self.progress >= 1.0 {
            return self.target.iter().collect();
        }

        let p = self.progress;
        let mut out = String::with_capacity(self.target.len());

        for (i, &c) in self.target.iter().enumerate() {
            let threshold = self.order.get(i).copied().unwrap_or(0.0);
            if p >= threshold {
                out.push(c);
                continue;
            }
            match self.kind {
                EffectKind::Decrypt => {
                    // Whitespace stays whitespace; scrambling it just makes the
                    // line look like noise.
                    if c.is_whitespace() {
                        out.push(c);
                    } else {
                        let idx = self.rng.below(SCRAMBLE.len());
                        out.push(SCRAMBLE[idx]);
                    }
                }
                EffectKind::Wipe | EffectKind::Scatter => out.push(' '),
                EffectKind::Slide => out.push(' '),
                EffectKind::None => out.push(c),
            }
        }

        // Slide additionally offsets the whole line.
        if self.kind == EffectKind::Slide {
            let shift = ((1.0 - p) * 8.0) as usize;
            let mut shifted = String::with_capacity(out.len() + shift);
            for _ in 0..shift {
                shifted.push(' ');
            }
            shifted.push_str(out.trim_start());
            return shifted;
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_effect_is_exactly_the_target_text() {
        let mut e = TextEffect::new(EffectKind::Decrypt, "Angra — Nova Era", 0.4, 1);
        e.advance(1.0);
        assert!(e.finished());
        assert_eq!(e.render(), "Angra — Nova Era");
    }

    #[test]
    fn the_none_effect_is_immediately_complete() {
        let mut e = TextEffect::new(EffectKind::None, "Hello", 0.4, 1);
        assert!(e.finished(), "nothing to animate");
        assert_eq!(e.render(), "Hello");
    }

    #[test]
    fn effects_converge_within_their_duration() {
        for kind in EffectKind::all() {
            let mut e = TextEffect::new(*kind, "Some Track Title", 0.4, 7);
            // Thirty frames at 30fps is one second, comfortably past 0.4s.
            for _ in 0..30 {
                e.advance(1.0 / 30.0);
            }
            assert!(e.finished(), "{kind:?} did not finish");
            assert_eq!(e.render(), "Some Track Title", "{kind:?}");
        }
    }

    #[test]
    fn output_keeps_the_same_character_count_while_animating() {
        // The title sits in a fixed-width row; a transition that changes length
        // would make the layout jitter.
        let mut e = TextEffect::new(EffectKind::Decrypt, "Angra - Nova Era", 0.4, 3);
        let want = "Angra - Nova Era".chars().count();
        for _ in 0..5 {
            e.advance(1.0 / 30.0);
            assert_eq!(e.render().chars().count(), want);
        }
    }

    #[test]
    fn decrypt_leaves_whitespace_alone() {
        let mut e = TextEffect::new(EffectKind::Decrypt, "a b c", 1.0, 5);
        e.advance(0.01);
        let r: Vec<char> = e.render().chars().collect();
        assert_eq!(r[1], ' ');
        assert_eq!(r[3], ' ');
    }

    #[test]
    fn wipe_reveals_left_to_right() {
        let mut e = TextEffect::new(EffectKind::Wipe, "abcdefgh", 1.0, 1);
        e.advance(0.5);
        let r = e.render();
        assert!(r.starts_with('a'), "left end should be revealed: {r:?}");
        assert!(r.ends_with(' '), "right end should not be: {r:?}");
    }

    #[test]
    fn finish_skips_to_the_end() {
        let mut e = TextEffect::new(EffectKind::Scatter, "abcdef", 5.0, 1);
        e.advance(0.1);
        assert!(!e.finished());
        e.finish();
        assert!(e.finished());
        assert_eq!(e.render(), "abcdef");
    }

    #[test]
    fn the_same_seed_gives_the_same_animation() {
        let run = || {
            let mut e = TextEffect::new(EffectKind::Decrypt, "abcdefgh", 1.0, 42);
            e.advance(0.2);
            e.render()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn effect_names_round_trip() {
        for k in EffectKind::all() {
            assert_eq!(EffectKind::parse(k.name()), Some(*k));
        }
        assert_eq!(EffectKind::parse("nonsense"), None);
    }
}
