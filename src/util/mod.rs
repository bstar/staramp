pub mod image;
pub mod rng;

/// Text from a file, a tag or another machine, made safe to print.
///
/// The TUI is protected already: ratatui drops control characters on the way
/// into its cell buffer, so an escape sequence in a track title cannot reach
/// the terminal through it. The command line has no such layer. `staramp
/// search`, `query`, `probe` and `playlists` print tag text, cue titles and
/// playlist lines straight to stdout, and none of that was written by this
/// program -- a title containing `\x1b[2J` clears the screen, and worse
/// sequences can retitle the window or stuff the input buffer.
///
/// Newlines survive, because the callers that want them are printing more than
/// one line on purpose and nothing else here emits them.
pub fn printable(s: &str) -> String {
    if !s.chars().any(|c| c.is_control() && c != '\n') {
        return s.to_string();
    }
    s.chars()
        .filter(|&c| !c.is_control() || c == '\n')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_never_reach_the_terminal() {
        assert_eq!(printable("Ira Sancti"), "Ira Sancti");
        assert_eq!(printable("clear\x1b[2Jme"), "clear[2Jme");
        assert_eq!(printable("title\x07"), "title");
        assert_eq!(printable("a\rb"), "ab");
        assert_eq!(printable("two\nlines"), "two\nlines");
        // Non-ASCII is text, not a control character.
        assert_eq!(printable("Mötley Crüe"), "Mötley Crüe");
    }
}
