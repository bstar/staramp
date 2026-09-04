//! The play queue: an order to walk, plus a play-next queue that jumps it.
//!
//! Two levels, because they answer different questions. The **order** is how the
//! playlist is traversed — sequential, or a shuffle of it. The **queue** is a
//! short list of tracks to play next regardless of where the order is.
//!
//! Advancing is transactional. A failed advance restores `pos`, the queue and
//! even `order`, so a queue full of unplayable tracks cannot leave the player in
//! a state where nothing works any more.

// `with_seed` is how the shuffle tests get a reproducible order, and
// `is_empty` and `play_next_len` are the questions a caller asks before
// deciding whether to draw anything. Tested, and waiting on the UI that asks.
#![allow(dead_code)]

use crate::util::rng::Lcg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    /// Read a mode by name, however it is cased. Anything unrecognised is off.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" => RepeatMode::All,
            "one" => RepeatMode::One,
            _ => RepeatMode::Off,
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

impl std::fmt::Display for RepeatMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RepeatMode::Off => "Off",
            RepeatMode::All => "All",
            RepeatMode::One => "One",
        })
    }
}

// Not `Eq`: ReplayGain is decibels, and floats have no total equality. Nothing
// keys a map on a queue item.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueueItem {
    pub uri: super::uri::TrackUri,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Whose record it is, which is not always whose track it is: a
    /// compilation's tracks each have their own artist and one album artist.
    pub album_artist: Option<String>,
    pub year: Option<i64>,
    pub disc_no: Option<u32>,
    pub track_no: Option<u32>,
    pub duration_secs: Option<i64>,
    /// ReplayGain as tagged, in dB, with the peaks that go with it.
    ///
    /// Carried on the item rather than looked up when a track opens: the queue
    /// is already where the index's answers live, and the decode thread has no
    /// database handle of its own.
    #[serde(default)]
    pub rg: crate::audio::dsp::gain::ReplayGain,
    /// Known to be missing or undecodable. Skipped when advancing rather than
    /// removed, so the playlist still round-trips.
    pub unplayable: bool,
}

impl QueueItem {
    pub fn new(uri: super::uri::TrackUri) -> Self {
        Self {
            uri,
            title: None,
            artist: None,
            album: None,
            album_artist: None,
            year: None,
            disc_no: None,
            track_no: None,
            duration_secs: None,
            rg: Default::default(),
            unplayable: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    tracks: Vec<QueueItem>,
    /// Indices into `tracks`, in playback order.
    order: Vec<usize>,
    /// Position within `order`.
    pos: usize,
    shuffle: bool,
    /// `Some(descending)` while the queue is in album order.
    ///
    /// Held rather than applied once, because `set_tracks` rebuilds `order`
    /// from nothing on every playlist load and would otherwise throw an
    /// externally installed order away. Shuffle *overrides* this while it is
    /// on; turning shuffle off brings the albums back rather than the raw
    /// playlist order.
    group: Option<bool>,
    /// Records arranged by hand, by album title, in the order they should
    /// play. Empty means the year order stands.
    manual: Vec<String>,
    repeat: RepeatMode,
    /// Play-next: track indices that pre-empt the order.
    play_next: Vec<usize>,
    /// Set while the current track came from `play_next`.
    from_queue: bool,
    rng: Lcg,
    /// Draw a fresh seed before every shuffle.
    ///
    /// Off only for `with_seed`, so tests stay reproducible. A player that
    /// shuffles the same way every launch is not shuffling.
    reseed: bool,
    /// Bumped on every real change, so observers can diff cheaply.
    revision: u64,
}

impl Queue {
    pub fn new() -> Self {
        Self {
            rng: Lcg::from_entropy(),
            reseed: true,
            ..Default::default()
        }
    }

    /// A queue with a fixed shuffle sequence, for tests.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            rng: Lcg::new(seed),
            reseed: false,
            ..Default::default()
        }
    }

    /// Draw a fresh seed, unless this queue was created for reproducibility.
    ///
    /// Seeding once at construction is not enough: the *first* shuffle after
    /// launch would be identical every run, which is the one people notice.
    fn reseed(&mut self) {
        if self.reseed {
            self.rng = Lcg::from_entropy();
        }
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }
    pub fn shuffled(&self) -> bool {
        self.shuffle
    }
    /// `Some(descending)` while the queue is in album order.
    ///
    /// Remembered even while shuffle is overriding it, so turning shuffle off
    /// brings the albums back. Ask [`grouped_now`](Self::grouped_now) for
    /// whether the order on screen is actually grouped.
    pub fn grouping(&self) -> Option<bool> {
        self.group
    }

    /// The hand-made album order, empty when there is none.
    pub fn manual_order(&self) -> &[String] {
        &self.manual
    }

    /// Arrange the records by hand, or hand them back to the year order.
    ///
    /// Does not interrupt what is playing: the track keeps playing and `pos`
    /// follows it to wherever its record lands.
    pub fn set_manual_order(&mut self, order: Vec<String>) {
        if self.manual == order {
            return;
        }
        self.manual = order;
        if !self.shuffle {
            self.resequence();
        }
        self.revision += 1;
    }

    /// Whether the order *is* grouped right now, rather than set to be.
    ///
    /// What the list should draw dividers along. Breaking a shuffled list on
    /// changes of album puts a heading every second row, which is noise rather
    /// than structure.
    pub fn grouped_now(&self) -> bool {
        self.group.is_some() && !self.shuffle
    }
    pub fn tracks(&self) -> &[QueueItem] {
        &self.tracks
    }
    pub fn play_next_len(&self) -> usize {
        self.play_next.len()
    }

    /// The order the playlist should be *shown* in.
    ///
    /// Returns indices into `tracks`. When shuffle is on this is the shuffled
    /// order, so the list on screen is the order things will actually play in.
    /// Rendering `tracks` directly instead made shuffle invisible: the internal
    /// order changed but the list looked identical every time, which reads as
    /// shuffle being broken.
    pub fn view(&self) -> &[usize] {
        &self.order
    }

    /// Where a track sits in the displayed order.
    pub fn view_position(&self, track_index: usize) -> Option<usize> {
        self.order.iter().position(|&i| i == track_index)
    }

    /// Position within the displayed order of what is playing.
    pub fn view_cursor(&self) -> usize {
        self.pos
    }

    /// Index into `tracks` of what is playing.
    pub fn current_index(&self) -> Option<usize> {
        self.order.get(self.pos).copied()
    }

    pub fn current(&self) -> Option<&QueueItem> {
        self.tracks.get(self.current_index()?)
    }

    /// Position within the playlist, 1-based, for display.
    pub fn position(&self) -> usize {
        self.pos + 1
    }

    pub fn set_tracks(&mut self, tracks: Vec<QueueItem>) {
        self.tracks = tracks;
        self.order = (0..self.tracks.len()).collect();
        self.pos = 0;
        self.play_next.clear();
        self.from_queue = false;
        if self.shuffle {
            self.reshuffle_pinning_current();
        } else {
            self.resequence();
            // A fresh queue starts at the top of its new order. `resequence`
            // keeps `pos` on the playing track, and here that is track zero of
            // the list being replaced -- which in album order is somewhere in
            // the middle.
            self.pos = 0;
        }
        self.revision += 1;
    }

    /// Reload an edited playlist while keeping the current decoder's place.
    pub fn refresh_tracks(&mut self, tracks: Vec<QueueItem>) {
        let current = self.current_index().and_then(|index| {
            let uri = self.tracks.get(index)?.uri.to_string();
            let occurrence = self.tracks[..=index]
                .iter()
                .filter(|item| item.uri.to_string() == uri)
                .count()
                .saturating_sub(1);
            Some((uri, occurrence))
        });
        let old_position = self.pos;
        self.tracks = tracks;
        self.order = (0..self.tracks.len()).collect();
        self.play_next.clear();
        self.from_queue = false;
        self.pos = current
            .as_ref()
            .and_then(|(uri, occurrence)| {
                self.tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.uri.to_string() == *uri)
                    .nth(*occurrence)
                    .map(|(index, _)| index)
            })
            .unwrap_or_else(|| old_position.min(self.tracks.len().saturating_sub(1)));
        if self.shuffle {
            self.reshuffle_pinning_current();
        } else {
            self.resequence();
        }
        self.revision += 1;
    }

    /// Put the queue in album order, or take it out of one.
    ///
    /// Does not interrupt what is playing: the track keeps playing and `pos`
    /// follows it to wherever it lands. What changes is what comes next.
    pub fn set_grouping(&mut self, group: Option<bool>) {
        if self.group == group {
            return;
        }
        self.group = group;
        // Shuffle owns `order` while it is on, so the new grouping is simply
        // remembered until shuffle is turned off again.
        if !self.shuffle {
            self.resequence();
        }
        self.revision += 1;
    }

    /// The order when nothing is shuffling it: grouped if a grouping is set,
    /// storage order if not. Keeps `pos` on whatever is playing.
    ///
    /// With no grouping this produces the identity, and `view_position` of a
    /// track is then its own index -- which is exactly what the code this
    /// replaced did by hand.
    fn resequence(&mut self) {
        let current = self.current_index();
        self.order = match self.group {
            Some(descending) => super::group::album_order(&self.tracks, descending, &self.manual),
            None => (0..self.tracks.len()).collect(),
        };
        self.pos = current.and_then(|c| self.view_position(c)).unwrap_or(0);
    }

    /// Append a track.
    ///
    /// Lands at the end of the order even when the queue is grouped, rather
    /// than inside its own album. Nothing calls this outside these tests, and
    /// resequencing here would move the listener's place in a grouped queue
    /// for the sake of a track they just added to the end on purpose.
    pub fn push(&mut self, item: QueueItem) {
        self.tracks.push(item);
        self.order.push(self.tracks.len() - 1);
        self.revision += 1;
    }

    /// Replace one storage row without disturbing playback or ordering.
    ///
    /// Used when an on-disk playlist replaces an alternate encoding in place.
    /// Indices stay valid, including the current and play-next pointers.
    pub fn replace_at(&mut self, index: usize, item: QueueItem) -> bool {
        let Some(slot) = self.tracks.get_mut(index) else {
            return false;
        };
        *slot = item;
        self.revision += 1;
        true
    }

    /// Rebuild `tracks` from `keep`, and carry everything that points into it.
    ///
    /// The one place that may permute `tracks`, and the reason the whole family
    /// below shares it. Three things point at track indices and every one of
    /// them is a silent bug if it is left behind:
    ///
    /// * `order` — a stale index makes `tracks.get(i)` return `None`, which the
    ///   panel and the `queue` verb both `filter_map` away without a word, so
    ///   the row coordinates quietly stop matching the queue's.
    /// * `play_next` — holds raw track indices, so a shift makes every queued
    ///   entry name a different song. Playable, and wrong.
    /// * `pos` — indexes `order`, and has to end up on the same *track* it was
    ///   on, not the same position.
    ///
    /// `keep` is a map from old index to new, `None` for a track that is going.
    fn rebuild(&mut self, keep: &[Option<usize>], tracks: Vec<QueueItem>) {
        let playing = self.current_index();
        self.tracks = tracks;
        self.order = self.order.iter().filter_map(|&i| keep[i]).collect();
        self.play_next = self.play_next.iter().filter_map(|&i| keep[i]).collect();
        // The same follow-the-track step `resequence` takes, but landing on the
        // neighbour rather than the top of the list when the track is gone --
        // `unwrap_or(0)` is right for a reorder, where nothing is ever lost.
        self.pos = match playing.and_then(|t| keep[t]) {
            Some(t) => self.view_position(t).unwrap_or(0),
            None => self.pos.min(self.order.len().saturating_sub(1)),
        };
        self.revision += 1;
        debug_assert!(
            super::group::is_permutation(&self.order, self.tracks.len()),
            "the order stopped being a permutation of the tracks"
        );
    }

    /// Take rows out of the list as it is shown.
    ///
    /// `rows` are view positions. Out of range and repeated rows are ignored
    /// rather than refused -- they can arrive from another window, and a
    /// request that half-applies is worse than one that quietly asks for less.
    ///
    /// `protect_playing` keeps the row at the play cursor whatever else goes.
    /// It is a parameter rather than a rule for the reason
    /// [`set_shuffle_pinning`](Self::set_shuffle_pinning) takes its pin: with
    /// nothing playing there is no track to protect, and refusing to delete
    /// row one of a stopped queue is a rule nobody can see.
    ///
    /// Returns how many went.
    pub fn remove(&mut self, rows: &[usize], protect_playing: bool) -> usize {
        let mut rows: Vec<usize> = rows
            .iter()
            .copied()
            .filter(|&p| p < self.order.len())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        if protect_playing {
            rows.retain(|&p| p != self.pos);
        }
        let doomed: Vec<usize> = rows.iter().map(|&p| self.order[p]).collect();
        self.remove_tracks(&doomed)
    }

    /// The same, by index into `tracks`, for callers that already hold one.
    fn remove_tracks(&mut self, doomed: &[usize]) -> usize {
        let going: std::collections::HashSet<usize> = doomed
            .iter()
            .copied()
            .filter(|&i| i < self.tracks.len())
            .collect();
        if going.is_empty() {
            return 0;
        }
        let mut keep = vec![None; self.tracks.len()];
        let mut next = 0;
        for (i, slot) in keep.iter_mut().enumerate() {
            if !going.contains(&i) {
                *slot = Some(next);
                next += 1;
            }
        }
        let kept = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, _)| !going.contains(i))
            .map(|(_, t)| t.clone())
            .collect();
        self.rebuild(&keep, kept);
        going.len()
    }

    /// Put `items` into the list so they occupy view positions `at..`.
    ///
    /// Storage moves with them, but **only next to their new neighbour** --
    /// they are spliced in after whichever track the row above them holds.
    /// Rebuilding storage in view order instead would write the album grouping
    /// or the shuffle into `tracks`, and `tracks` is what a save writes: one
    /// paste into a shuffled queue would put the shuffle in the file for ever.
    ///
    /// Nothing is resequenced, for the same reason [`push`](Self::push) gives:
    /// re-sorting for the sake of a track the listener just placed by hand
    /// moves their place in the list.
    pub fn insert_at(&mut self, at: usize, items: Vec<QueueItem>) -> usize {
        let n = items.len();
        if n == 0 {
            return 0;
        }
        let row = at.min(self.order.len());
        // The storage slot the new rows belong beside: just after the track the
        // row above them holds, or the very top when there is no row above.
        let slot = match row.checked_sub(1).and_then(|p| self.order.get(p)) {
            Some(&t) => t + 1,
            None => 0,
        };

        // Everything naming a track at or after the slot shifts up by `n`.
        let shift = |i: &mut usize| {
            if *i >= slot {
                *i += n;
            }
        };
        self.order.iter_mut().for_each(shift);
        self.play_next.iter_mut().for_each(shift);

        self.tracks.splice(slot..slot, items);
        self.order.splice(row..row, slot..slot + n);
        if self.pos >= row {
            self.pos += n;
        }

        self.revision += 1;
        debug_assert!(
            super::group::is_permutation(&self.order, self.tracks.len()),
            "the order stopped being a permutation of the tracks"
        );
        n
    }

    /// Lift rows out and put them back at a position in the view.
    ///
    /// `rows` and `at` are both view positions -- the coordinate the cursor
    /// lives in and the only one shared with a window that is following this
    /// session. They arrive in the order they were shown in, so moving a
    /// scattered handful gathers them without shuffling them.
    pub fn move_to(&mut self, rows: &[usize], at: usize) -> usize {
        let mut rows: Vec<usize> = rows
            .iter()
            .copied()
            .filter(|&p| p < self.order.len())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        if rows.is_empty() {
            return 0;
        }
        let moving: Vec<QueueItem> = rows
            .iter()
            .map(|&p| self.tracks[self.order[p]].clone())
            .collect();
        // Where `at` lands once the rows above it have been taken out.
        let above = rows.iter().take_while(|&&p| p < at).count();
        let n = moving.len();
        let doomed: Vec<usize> = rows.iter().map(|&p| self.order[p]).collect();
        self.remove_tracks(&doomed);
        self.insert_at(at.saturating_sub(above), moving);
        n
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
        self.revision += 1;
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.cycle();
        self.revision += 1;
        self.repeat
    }

    /// Toggle shuffle without interrupting what is playing.
    /// Toggle shuffle without interrupting what is playing.
    pub fn set_shuffle(&mut self, on: bool) {
        self.set_shuffle_pinning(on, true)
    }

    /// As `set_shuffle`, but `pin` decides whether the current track is held at
    /// the front of the new order.
    ///
    /// Pinning exists so turning shuffle on *mid-song* does not interrupt the
    /// song. With nothing playing there is no song to protect, and pinning then
    /// forces every shuffle to start on the same track -- which reads as
    /// shuffle not working at all.
    pub fn set_shuffle_pinning(&mut self, on: bool, pin: bool) {
        if self.shuffle == on {
            return;
        }
        self.shuffle = on;
        if on {
            if pin {
                self.reshuffle_pinning_current();
            } else {
                self.shuffle_now();
            }
        } else {
            // Back to whatever the unshuffled order is -- the albums, if a
            // grouping is set -- with the cursor still on the same track.
            //
            // `current` is a *track* index and `pos` a position in `order`.
            // Assigning one to the other was right only while this branch
            // rebuilt the identity; `resequence` looks the track up instead.
            self.resequence();
        }
        self.revision += 1;
    }

    pub fn toggle_shuffle(&mut self) -> bool {
        self.toggle_shuffle_pinning(true)
    }

    pub fn toggle_shuffle_pinning(&mut self, pin: bool) -> bool {
        self.set_shuffle_pinning(!self.shuffle, pin);
        self.shuffle
    }

    /// Fisher-Yates over everything *except* the current track, which is pinned
    /// to the front.
    ///
    /// Pinning is what makes turning shuffle on mid-song not interrupt it — the
    /// alternative silently jumps to a different track, which reads as a bug.
    fn reshuffle_pinning_current(&mut self) {
        self.reseed();
        let current = self.current_index();
        let mut rest: Vec<usize> = (0..self.tracks.len())
            .filter(|i| Some(*i) != current)
            .collect();

        for i in (1..rest.len()).rev() {
            let j = self.rng.below(i + 1);
            rest.swap(i, j);
        }

        self.order = match current {
            Some(c) => std::iter::once(c).chain(rest).collect(),
            None => rest,
        };
        self.pos = 0;
    }

    /// Shuffle everything and start from a fresh first track.
    ///
    /// Distinct from `set_shuffle(true)`, which pins whatever is playing to the
    /// front so turning shuffle on mid-song does not interrupt it. This is the
    /// "shuffle and go" gesture: nothing is pinned, so it genuinely lands
    /// somewhere new. Returns the track to play.
    pub fn shuffle_now(&mut self) -> Option<usize> {
        if self.tracks.is_empty() {
            return None;
        }
        self.reseed();
        self.shuffle = true;
        self.play_next.clear();
        self.from_queue = false;

        let mut order: Vec<usize> = (0..self.tracks.len()).collect();
        for i in (1..order.len()).rev() {
            let j = self.rng.below(i + 1);
            order.swap(i, j);
        }
        self.order = order;

        // Land on the first playable slot rather than blindly on slot 0.
        self.pos = self
            .order
            .iter()
            .position(|&i| self.is_playable(i))
            .unwrap_or(0);
        self.revision += 1;
        self.current_index()
    }

    /// Add to the play-next queue.
    pub fn queue_next(&mut self, track_index: usize) {
        if track_index < self.tracks.len() && !self.play_next.contains(&track_index) {
            self.play_next.push(track_index);
            self.revision += 1;
        }
    }

    pub fn clear_play_next(&mut self) {
        self.play_next.clear();
        self.revision += 1;
    }

    fn is_playable(&self, track_index: usize) -> bool {
        self.tracks
            .get(track_index)
            .map(|t| !t.unplayable)
            .unwrap_or(false)
    }

    /// Advance. Returns the new current track index, or `None` if there is
    /// nowhere to go — in which case nothing has been mutated.
    pub fn next(&mut self) -> Option<usize> {
        let snapshot = (
            self.pos,
            self.order.clone(),
            self.play_next.clone(),
            self.from_queue,
        );

        // 1. The play-next queue wins, taking the first playable entry.
        while let Some(&head) = self.play_next.first() {
            self.play_next.remove(0);
            if self.is_playable(head) {
                // Point `pos` at it so the order stays coherent afterwards.
                if let Some(p) = self.order.iter().position(|&i| i == head) {
                    self.pos = p;
                }
                self.from_queue = true;
                self.revision += 1;
                return Some(head);
            }
        }
        self.from_queue = false;

        // 2. Repeat-one replays the current track.
        if self.repeat == RepeatMode::One {
            if let Some(c) = self.current_index() {
                if self.is_playable(c) {
                    self.revision += 1;
                    return Some(c);
                }
            }
        }

        // 3. Walk forward for the next playable slot.
        if let Some(found) = self.advance_forward() {
            self.revision += 1;
            return Some(found);
        }

        // 4. Wrap, if repeating.
        if self.repeat == RepeatMode::All && !self.tracks.is_empty() {
            if self.shuffle {
                // A fresh shuffle each lap, rather than replaying one order
                // forever.
                self.reshuffle_pinning_current();
            }
            self.pos = 0;
            if self.is_playable_at_pos() {
                self.revision += 1;
                return self.current_index();
            }
            if let Some(found) = self.advance_forward() {
                self.revision += 1;
                return Some(found);
            }
        }

        // Nothing playable anywhere: restore exactly what we started with.
        self.pos = snapshot.0;
        self.order = snapshot.1;
        self.play_next = snapshot.2;
        self.from_queue = snapshot.3;
        None
    }

    fn is_playable_at_pos(&self) -> bool {
        self.current_index()
            .map(|i| self.is_playable(i))
            .unwrap_or(false)
    }

    fn advance_forward(&mut self) -> Option<usize> {
        let mut p = self.pos;
        while p + 1 < self.order.len() {
            p += 1;
            let idx = self.order[p];
            if self.is_playable(idx) {
                self.pos = p;
                return Some(idx);
            }
        }
        None
    }

    /// Step back to the previous playable track.
    pub fn prev(&mut self) -> Option<usize> {
        let start = self.pos;
        let mut p = self.pos;
        while p > 0 {
            p -= 1;
            let idx = self.order[p];
            if self.is_playable(idx) {
                self.pos = p;
                self.revision += 1;
                return Some(idx);
            }
        }
        self.pos = start;
        None
    }

    /// Follow a position in the order without calling it a change.
    ///
    /// For a client tracking the instance that owns playback: it is told where
    /// the order has got to several times a second, and `jump_to` -- which is
    /// right for a jump somebody asked for -- bumps `revision` every time it is
    /// called. Observers key their caches on that revision, so following along
    /// invalidated them thirty times a second.
    pub fn set_view_cursor(&mut self, pos: usize) {
        if pos < self.order.len() {
            self.pos = pos;
        }
    }

    /// Jump straight to a track index.
    pub fn jump_to(&mut self, track_index: usize) -> Option<usize> {
        let p = self.order.iter().position(|&i| i == track_index)?;
        self.pos = p;
        self.from_queue = false;
        self.revision += 1;
        Some(track_index)
    }

    /// What would play next, without advancing. Used to prime the gapless
    /// preload. Deliberately gives up at a shuffle wrap, where the next track is
    /// genuinely not yet decided.
    pub fn peek_next(&self) -> Option<usize> {
        if let Some(&head) = self.play_next.iter().find(|&&i| self.is_playable(i)) {
            return Some(head);
        }
        if self.repeat == RepeatMode::One {
            return self.current_index();
        }
        let mut p = self.pos;
        while p + 1 < self.order.len() {
            p += 1;
            if self.is_playable(self.order[p]) {
                return Some(self.order[p]);
            }
        }
        if self.repeat == RepeatMode::All && !self.shuffle {
            return self.order.iter().copied().find(|&i| self.is_playable(i));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::uri::TrackUri;

    fn queue_of(n: usize) -> Queue {
        let mut q = Queue::with_seed(42);
        q.set_tracks(
            (0..n)
                .map(|i| {
                    QueueItem::new(TrackUri::File {
                        rel_path: format!("t{i}.flac"),
                    })
                })
                .collect(),
        );
        q
    }

    /// Three albums, three tracks each, deliberately out of year order.
    fn albums() -> Queue {
        let mut q = Queue::with_seed(42);
        let mut items = Vec::new();
        for (album, year) in [("Chained", 2005), ("Holy Land", 1996), ("Fireworks", 1998)] {
            for track in 1..=3u32 {
                let mut it = QueueItem::new(TrackUri::File {
                    rel_path: format!("{album}/{track}.flac"),
                });
                it.album = Some(album.into());
                it.year = Some(year);
                it.track_no = Some(track);
                items.push(it);
            }
        }
        q.set_tracks(items);
        q
    }

    /// The album of each track, in the order the queue would play them.
    fn played(q: &Queue) -> Vec<String> {
        let mut v: Vec<String> = q
            .view()
            .iter()
            .map(|&i| q.tracks()[i].album.clone().unwrap())
            .collect();
        v.dedup();
        v
    }

    #[test]
    fn grouping_puts_the_albums_in_year_order_and_back_again() {
        let mut q = albums();
        assert_eq!(played(&q), ["Chained", "Holy Land", "Fireworks"]);

        q.set_grouping(Some(false));
        assert_eq!(played(&q), ["Holy Land", "Fireworks", "Chained"]);
        q.set_grouping(Some(true));
        assert_eq!(played(&q), ["Chained", "Fireworks", "Holy Land"]);

        q.set_grouping(None);
        assert_eq!(played(&q), ["Chained", "Holy Land", "Fireworks"]);
    }

    #[test]
    fn grouping_does_not_interrupt_what_is_playing() {
        // The track keeps playing and `pos` follows it. What changes is what
        // comes next -- which is the whole point of reordering the queue
        // rather than only the view.
        let mut q = albums();
        q.next();
        q.next();
        q.next();
        let playing = q.current_index().unwrap();
        q.set_grouping(Some(false));
        assert_eq!(q.current_index(), Some(playing), "jumped to another track");
        assert_eq!(q.view()[q.view_cursor()], playing, "pos lost its track");
    }

    #[test]
    fn the_grouping_survives_a_new_playlist() {
        // `set_tracks` rebuilds the order from nothing on every load, which is
        // why the mode is held here rather than installed from outside.
        let mut q = albums();
        q.set_grouping(Some(false));
        let items: Vec<QueueItem> = q.tracks().to_vec();
        q.set_tracks(items);
        assert_eq!(played(&q), ["Holy Land", "Fireworks", "Chained"]);
        assert_eq!(q.view_cursor(), 0, "a fresh queue starts at the top");
    }

    #[test]
    fn shuffle_overrides_the_grouping_and_gives_it_back() {
        let mut q = albums();
        q.set_grouping(Some(false));
        q.set_shuffle(true);
        assert!(q.shuffled());
        assert_eq!(q.grouping(), Some(false), "the mode is remembered");
        q.set_shuffle(false);
        assert_eq!(
            played(&q),
            ["Holy Land", "Fireworks", "Chained"],
            "shuffling off should give the albums back, not the raw order"
        );
    }

    #[test]
    fn turning_shuffle_off_lands_on_the_track_that_was_playing() {
        // `pos` is a position in the order and `current_index` a track. The
        // two are only interchangeable while the order is the identity, and a
        // grouped queue is exactly when it is not.
        let mut q = albums();
        q.set_grouping(Some(false));
        q.next();
        q.next();
        let playing = q.current_index().unwrap();
        q.set_shuffle(true);
        q.set_shuffle(false);
        assert_eq!(q.current_index(), Some(playing));
    }

    #[test]
    fn a_grouped_queue_wraps_to_the_oldest_album() {
        let mut q = albums();
        q.set_grouping(Some(false));
        q.set_repeat(RepeatMode::All);
        while q.view_cursor() + 1 < q.len() {
            q.next();
        }
        let last = q.current_index().unwrap();
        assert_eq!(
            q.tracks()[last].album.as_deref(),
            Some("Chained"),
            "the newest record should be at the end"
        );
        let wrapped = q.next().unwrap();
        assert_eq!(q.tracks()[wrapped].album.as_deref(), Some("Holy Land"));
        assert_eq!(q.view_cursor(), 0);
    }

    #[test]
    fn following_along_is_not_a_change_but_jumping_is() {
        // Observers key their caches on the revision. A client is told where
        // the playing instance has got to several times a second, and doing
        // that through `jump_to` moved the revision every frame -- so every
        // cache downstream rebuilt every frame.
        let mut q = albums();
        let before = q.revision();
        q.set_view_cursor(4);
        assert_eq!(q.view_cursor(), 4, "it still follows");
        assert_eq!(q.revision(), before, "and says nothing changed");

        // A jump somebody asked for is a change, and still says so.
        q.jump_to(2);
        assert!(q.revision() > before);

        // Out of range is ignored rather than panicking or wrapping.
        let now = q.view_cursor();
        q.set_view_cursor(9_999);
        assert_eq!(q.view_cursor(), now);
    }

    #[test]
    fn grouping_moves_the_revision_so_observers_notice() {
        let mut q = albums();
        let before = q.revision();
        q.set_grouping(Some(false));
        assert!(q.revision() > before);
        let after = q.revision();
        q.set_grouping(Some(false));
        assert_eq!(q.revision(), after, "setting it to what it already is");
    }

    #[test]
    fn advances_sequentially_and_stops_at_the_end() {
        let mut q = queue_of(3);
        assert_eq!(q.current_index(), Some(0));
        assert_eq!(q.next(), Some(1));
        assert_eq!(q.next(), Some(2));
        assert_eq!(q.next(), None, "no repeat, so it stops");
        assert_eq!(q.current_index(), Some(2), "and stays put");
    }

    #[test]
    fn repeat_all_wraps() {
        let mut q = queue_of(3);
        q.set_repeat(RepeatMode::All);
        q.next();
        q.next();
        assert_eq!(q.next(), Some(0), "wraps to the start");
    }

    #[test]
    fn repeat_one_replays_the_same_track() {
        let mut q = queue_of(3);
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.next(), Some(0));
        assert_eq!(q.next(), Some(0));
    }

    #[test]
    fn play_next_pre_empts_the_order() {
        let mut q = queue_of(5);
        q.queue_next(4);
        q.queue_next(2);
        assert_eq!(q.next(), Some(4));
        assert_eq!(q.next(), Some(2));
        // Queue drained; back to walking the order from wherever we landed.
        assert_eq!(q.next(), Some(3));
    }

    #[test]
    fn unplayable_tracks_are_skipped_not_stumbled_over() {
        let mut q = queue_of(4);
        q.tracks[1].unplayable = true;
        q.tracks[2].unplayable = true;
        assert_eq!(q.next(), Some(3), "skips both");
    }

    #[test]
    fn a_failed_advance_changes_nothing() {
        let mut q = queue_of(3);
        q.tracks[1].unplayable = true;
        q.tracks[2].unplayable = true;
        let before = (q.pos, q.order.clone(), q.revision());
        assert_eq!(q.next(), None);
        assert_eq!(
            (q.pos, q.order.clone(), q.revision()),
            before,
            "state must be exactly as it was"
        );
    }

    #[test]
    fn a_queue_of_only_unplayable_entries_does_not_wedge_the_player() {
        let mut q = queue_of(3);
        q.tracks[2].unplayable = true;
        q.queue_next(2);
        // The queued entry is unplayable, so it is discarded and the order is
        // used instead.
        assert_eq!(q.next(), Some(1));
    }

    #[test]
    fn enabling_shuffle_does_not_interrupt_the_current_track() {
        let mut q = queue_of(20);
        q.next();
        q.next();
        let playing = q.current_index();
        q.set_shuffle(true);
        assert_eq!(q.current_index(), playing, "the pinned track keeps playing");
    }

    #[test]
    fn disabling_shuffle_keeps_the_current_track_and_restores_order() {
        let mut q = queue_of(10);
        q.set_shuffle(true);
        q.next();
        let playing = q.current_index().unwrap();
        q.set_shuffle(false);
        assert_eq!(q.current_index(), Some(playing));
        assert_eq!(q.order, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_visits_every_track_exactly_once() {
        let mut q = queue_of(50);
        q.set_shuffle(true);
        let mut seen = vec![q.current_index().unwrap()];
        while let Some(i) = q.next() {
            seen.push(i);
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 50, "no track skipped or repeated");
    }

    #[test]
    fn the_view_follows_the_play_order_when_shuffled() {
        let mut q = queue_of(20);
        assert_eq!(
            q.view(),
            (0..20).collect::<Vec<_>>(),
            "sequential by default"
        );

        q.shuffle_now();
        assert_ne!(
            q.view(),
            (0..20).collect::<Vec<_>>(),
            "shuffling must change what is shown, or it looks broken"
        );
        // Still every track, exactly once.
        let mut sorted = q.view().to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn turning_shuffle_off_restores_the_shown_order() {
        let mut q = queue_of(20);
        q.shuffle_now();
        q.set_shuffle(false);
        assert_eq!(q.view(), (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn the_view_cursor_tracks_what_is_playing() {
        let mut q = queue_of(20);
        q.shuffle_now();
        for _ in 0..5 {
            q.next();
        }
        let showing = q.view()[q.view_cursor()];
        assert_eq!(Some(showing), q.current_index());
    }

    #[test]
    fn a_default_queue_shuffles_differently_every_time() {
        // The bug this guards: a hardcoded seed meant the first shuffle after
        // every launch produced the identical order.
        let order_of = || {
            let mut q = Queue::new();
            q.set_tracks(
                (0..40)
                    .map(|i| {
                        QueueItem::new(TrackUri::File {
                            rel_path: format!("t{i}.flac"),
                        })
                    })
                    .collect(),
            );
            q.shuffle_now();
            let mut seen = vec![q.current_index().unwrap()];
            while let Some(i) = q.next() {
                seen.push(i);
            }
            seen
        };
        let runs: Vec<Vec<usize>> = (0..4).map(|_| order_of()).collect();
        assert!(
            runs.windows(2).any(|w| w[0] != w[1]),
            "four fresh queues all shuffled identically"
        );
    }

    #[test]
    fn a_seeded_queue_stays_reproducible_for_tests() {
        let order_of = || {
            let mut q = queue_of(30);
            q.shuffle_now();
            q.order.clone()
        };
        assert_eq!(order_of(), order_of());
    }

    #[test]
    fn successive_shuffles_in_one_session_differ() {
        let mut q = Queue::new();
        q.set_tracks(
            (0..40)
                .map(|i| {
                    QueueItem::new(TrackUri::File {
                        rel_path: format!("t{i}.flac"),
                    })
                })
                .collect(),
        );
        q.shuffle_now();
        let first = q.order.clone();
        q.shuffle_now();
        assert_ne!(first, q.order, "shuffling twice gave the same order");
    }

    #[test]
    fn shuffle_now_reorders_without_pinning_the_current_track() {
        let mut q = queue_of(60);
        // Walk a few tracks in so "current" is not already slot 0.
        q.next();
        q.next();
        let before = q.current_index();

        let landed = q.shuffle_now();
        assert!(q.shuffled());
        assert!(landed.is_some());
        // set_shuffle pins the current track; shuffle_now must not.
        assert_ne!(
            landed, before,
            "shuffle_now should land somewhere new, not pin what was playing"
        );
    }

    #[test]
    fn shuffle_now_still_covers_every_track_once() {
        let mut q = queue_of(40);
        q.shuffle_now();
        let mut seen = vec![q.current_index().unwrap()];
        while let Some(i) = q.next() {
            seen.push(i);
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 40);
    }

    #[test]
    fn shuffle_now_skips_unplayable_tracks_when_choosing_where_to_land() {
        let mut q = queue_of(10);
        for i in 0..10 {
            q.tracks[i].unplayable = i != 7;
        }
        assert_eq!(q.shuffle_now(), Some(7), "only track 7 is playable");
    }

    #[test]
    fn shuffle_now_on_an_empty_queue_does_nothing() {
        let mut q = Queue::with_seed(1);
        assert_eq!(q.shuffle_now(), None);
    }

    #[test]
    fn shuffle_now_clears_a_stale_play_next_queue() {
        let mut q = queue_of(20);
        q.queue_next(5);
        q.shuffle_now();
        assert_eq!(q.play_next_len(), 0, "a fresh shuffle discards play-next");
    }

    #[test]
    fn peek_next_agrees_with_next() {
        let mut q = queue_of(5);
        for _ in 0..4 {
            let peeked = q.peek_next();
            let actual = q.next();
            assert_eq!(peeked, actual);
        }
    }

    #[test]
    fn peek_next_declines_to_guess_at_a_shuffle_wrap() {
        let mut q = queue_of(3);
        q.set_shuffle(true);
        q.set_repeat(RepeatMode::All);
        q.next();
        q.next();
        assert_eq!(
            q.peek_next(),
            None,
            "the next lap is not decided until it is shuffled"
        );
    }

    #[test]
    fn repeat_cycles_off_all_one() {
        let mut q = queue_of(1);
        assert_eq!(q.cycle_repeat(), RepeatMode::All);
        assert_eq!(q.cycle_repeat(), RepeatMode::One);
        assert_eq!(q.cycle_repeat(), RepeatMode::Off);
    }
    fn named(names: &[&str]) -> Vec<QueueItem> {
        names
            .iter()
            .map(|n| {
                QueueItem::new(super::super::uri::TrackUri::File {
                    rel_path: (*n).into(),
                })
            })
            .collect()
    }

    fn shown(q: &Queue) -> Vec<String> {
        q.view()
            .iter()
            .map(|&i| q.tracks()[i].uri.to_string())
            .collect()
    }

    /// The contract `group.rs` states: an order that dropped an index would not
    /// hide a track, it would make it unreachable.
    #[test]
    fn removing_a_row_leaves_the_order_a_permutation_of_what_is_left() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c", "d", "e"]));
        assert_eq!(q.remove(&[1, 3], false), 2);
        assert_eq!(q.len(), 3);
        assert_eq!(shown(&q), ["a", "c", "e"]);
        assert!(super::super::group::is_permutation(q.view(), q.len()));
    }

    #[test]
    fn refreshing_tracks_keeps_the_playing_uri_selected() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        q.jump_to(1);
        q.refresh_tracks(named(&["x", "a", "b", "c"]));
        assert_eq!(q.current().unwrap().uri.to_string(), "b");
        assert!(super::super::group::is_permutation(q.view(), q.len()));
    }

    #[test]
    fn replacing_a_storage_row_does_not_move_the_playing_position() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "old.mp3", "c"]));
        q.jump_to(2);
        assert!(q.replace_at(1, named(&["new.flac"]).remove(0)));
        assert_eq!(shown(&q), ["a", "new.flac", "c"]);
        assert_eq!(q.current().unwrap().uri.to_string(), "c");
    }

    #[test]
    fn removing_a_row_above_the_playing_one_leaves_the_same_track_playing() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c", "d"]));
        q.jump_to(2);
        assert_eq!(q.current().unwrap().uri.to_string(), "c");
        q.remove(&[0], false);
        assert_eq!(
            q.current().unwrap().uri.to_string(),
            "c",
            "the listener was moved to a different song"
        );
    }

    /// `play_next` holds raw track indices. Shift them and every queued entry
    /// names a different song -- playable, and wrong.
    #[test]
    fn a_queued_track_still_names_its_own_song_after_a_removal_above_it() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c", "d"]));
        q.queue_next(3); // "d"
        q.remove(&[0], false); // everything above shifts down
        assert_eq!(q.play_next_len(), 1);
        assert_eq!(q.next().unwrap(), 2, "the queued track moved to index 2");
        assert_eq!(q.current().unwrap().uri.to_string(), "d");
    }

    #[test]
    fn removing_a_track_that_was_queued_forgets_it_rather_than_playing_another() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        q.queue_next(2);
        q.remove(&[2], false);
        assert_eq!(q.play_next_len(), 0, "a queued ghost survived");
    }

    #[test]
    fn removing_the_playing_track_lands_on_its_neighbour_not_the_top() {
        // `resequence` answers `unwrap_or(0)`, which is right for a reorder --
        // nothing is ever lost there. Losing a track is a different question.
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c", "d"]));
        q.jump_to(2);
        q.remove(&[2], false);
        assert_eq!(q.current().unwrap().uri.to_string(), "d");
    }

    #[test]
    fn the_playing_row_stays_when_it_is_protected() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        q.jump_to(1);
        assert_eq!(q.remove(&[0, 1, 2], true), 2, "the playing row should stay");
        assert_eq!(shown(&q), ["b"]);
        assert_eq!(q.current().unwrap().uri.to_string(), "b");
    }

    #[test]
    fn with_nothing_playing_every_row_asked_for_goes() {
        // The protection is the caller's to decide. Making it a rule inside the
        // queue would leave row one of a stopped playlist undeletable, for a
        // reason nobody could see -- `pos` is 0 whether or not anything plays.
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        assert_eq!(q.remove(&[0, 1, 2], false), 3);
        assert!(q.is_empty());
    }

    #[test]
    fn removing_everything_leaves_an_empty_queue_rather_than_a_broken_one() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b"]));
        assert_eq!(q.remove(&[0, 1], false), 2);
        assert!(q.is_empty());
        assert_eq!(q.current(), None);
        assert!(super::super::group::is_permutation(q.view(), q.len()));
    }

    #[test]
    fn a_removal_names_each_row_once_however_often_it_is_asked_for() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        assert_eq!(
            q.remove(&[1, 1, 9], false),
            1,
            "a repeat or a stray must not count"
        );
        assert_eq!(shown(&q), ["a", "c"]);
    }

    #[test]
    fn pasting_puts_the_rows_where_the_cursor_was_not_at_the_end() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        assert_eq!(q.insert_at(1, named(&["x", "y"])), 2);
        assert_eq!(shown(&q), ["a", "x", "y", "b", "c"]);
        // And in storage too, because that is what a saved playlist is written
        // from -- pasting into the middle and saving to the end is the bug this
        // whole arrangement exists to avoid.
        let stored: Vec<String> = q.tracks().iter().map(|t| t.uri.to_string()).collect();
        assert_eq!(stored, ["a", "x", "y", "b", "c"]);
    }

    /// A paste must not bake the *view* order into storage.
    ///
    /// Storage order is what a save writes. Album grouping and shuffle are
    /// view-only and must stay that way, or one paste into a shuffled queue
    /// would write the shuffle into the file for ever.
    #[test]
    fn pasting_does_not_write_the_shown_order_into_storage() {
        let mut q = Queue::with_seed(7);
        q.set_tracks(named(&["a", "b", "c", "d", "e", "f", "g", "h"]));
        q.set_shuffle(true);
        let before: Vec<String> = q.tracks().iter().map(|t| t.uri.to_string()).collect();
        assert_eq!(
            before,
            ["a", "b", "c", "d", "e", "f", "g", "h"],
            "storage starts in file order"
        );
        assert_ne!(shown(&q), before, "the seed should give a shuffled view");

        q.insert_at(0, named(&["x"]));

        let after: Vec<String> = q.tracks().iter().map(|t| t.uri.to_string()).collect();
        let without_x: Vec<&String> = after.iter().filter(|u| *u != "x").collect();
        assert_eq!(
            without_x,
            before.iter().collect::<Vec<_>>(),
            "the shuffle was written into storage"
        );
    }

    #[test]
    fn pasting_past_the_end_appends() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a"]));
        q.insert_at(99, named(&["z"]));
        assert_eq!(shown(&q), ["a", "z"]);
    }

    #[test]
    fn pasting_leaves_the_same_track_playing() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        q.jump_to(2);
        q.insert_at(0, named(&["x"]));
        assert_eq!(q.current().unwrap().uri.to_string(), "c");
    }

    #[test]
    fn moving_a_scattered_handful_gathers_them_without_shuffling_them() {
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c", "d", "e"]));
        // b and d, to the front. They keep the order they were already in.
        q.move_to(&[1, 3], 0);
        assert_eq!(shown(&q), ["b", "d", "a", "c", "e"]);
    }

    #[test]
    fn moving_rows_down_past_their_own_gap_lands_where_it_looks_like_it_should() {
        // The rows above the target come out first, so the target slides up by
        // however many of them there were. Getting this wrong puts them one
        // place out, which is the sort of thing nobody notices until it matters.
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c", "d", "e"]));
        q.move_to(&[0, 1], 4);
        assert_eq!(shown(&q), ["c", "d", "a", "b", "e"]);
    }

    #[test]
    fn every_edit_bumps_the_revision() {
        // A window following the session only refetches the queue when the
        // revision moves; without this it draws a list that no longer exists.
        let mut q = Queue::with_seed(1);
        q.set_tracks(named(&["a", "b", "c"]));
        let at = q.revision();
        q.remove(&[0], false);
        assert!(q.revision() > at, "a removal went unannounced");
        let at = q.revision();
        q.insert_at(0, named(&["x"]));
        assert!(q.revision() > at, "a paste went unannounced");
        let at = q.revision();
        q.move_to(&[0], 1);
        assert!(q.revision() > at, "a move went unannounced");
    }

    #[test]
    fn an_edit_under_album_order_keeps_the_view_and_the_storage_agreeing() {
        let mut q = Queue::with_seed(1);
        let mut items = named(&["a", "b", "c"]);
        items[0].album = Some("Later".into());
        items[0].year = Some(2010);
        items[1].album = Some("Early".into());
        items[1].year = Some(1990);
        items[2].album = Some("Later".into());
        items[2].year = Some(2010);
        q.set_tracks(items);
        q.set_grouping(Some(false));
        assert_eq!(shown(&q), ["b", "a", "c"], "album order, oldest first");
        // "b" is track 1 but view row 0 -- album order put it first.
        assert_eq!(q.view_position(1), Some(0));
        q.remove(&[0], false);
        assert_eq!(shown(&q), ["a", "c"]);
        assert!(super::super::group::is_permutation(q.view(), q.len()));
    }
}
