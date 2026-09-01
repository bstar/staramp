//! The query AST.
//!
//! One representation, two editors. The text query and the visual rule builder
//! both produce this, so they round-trip through each other losslessly instead
//! of being two implementations that drift apart.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Artist,
    AlbumArtist,
    Album,
    Title,
    Genre,
    Composer,
    Year,
    Codec,
    Bitrate,
    SampleRate,
    BitDepth,
    Duration,
    Path,
    FileSize,
    Added,
    PlayCount,
    SkipCount,
    LastPlayed,
    Rating,
    Loved,
    TrackNo,
    DiscNo,
    Lossless,
    Cue,
}

impl Field {
    /// Canonical name plus every accepted alias.
    pub fn parse(s: &str) -> Option<Field> {
        use Field::*;
        Some(match s.to_ascii_lowercase().as_str() {
            "artist" | "ar" | "a" => Artist,
            "albumartist" | "aa" => AlbumArtist,
            "album" | "al" => Album,
            "title" | "ti" | "t" => Title,
            "genre" | "g" => Genre,
            "composer" | "comp" => Composer,
            "year" | "y" => Year,
            "codec" | "format" | "fmt" => Codec,
            "bitrate" | "br" => Bitrate,
            "samplerate" | "sr" | "khz" => SampleRate,
            "bitdepth" | "bd" | "bits" => BitDepth,
            "duration" | "dur" | "length" | "len" => Duration,
            "path" | "file" | "dir" => Path,
            "filesize" | "size" => FileSize,
            "added" | "dateadded" | "imported" => Added,
            "playcount" | "pc" | "plays" => PlayCount,
            "skipcount" | "sc" | "skips" => SkipCount,
            "lastplayed" | "lp" | "played" => LastPlayed,
            "rating" | "stars" => Rating,
            "loved" | "fav" | "favorite" => Loved,
            "track" | "tn" | "trackno" => TrackNo,
            "disc" | "dn" | "discno" => DiscNo,
            "lossless" => Lossless,
            "cue" => Cue,
            _ => return None,
        })
    }

    pub fn canonical(self) -> &'static str {
        use Field::*;
        match self {
            Artist => "artist",
            AlbumArtist => "albumartist",
            Album => "album",
            Title => "title",
            Genre => "genre",
            Composer => "composer",
            Year => "year",
            Codec => "codec",
            Bitrate => "bitrate",
            SampleRate => "samplerate",
            BitDepth => "bitdepth",
            Duration => "duration",
            Path => "path",
            FileSize => "filesize",
            Added => "added",
            PlayCount => "playcount",
            SkipCount => "skipcount",
            LastPlayed => "lastplayed",
            Rating => "rating",
            Loved => "loved",
            TrackNo => "track",
            DiscNo => "disc",
            Lossless => "lossless",
            Cue => "cue",
        }
    }

    /// Every canonical name, for the rule builder's field picker and for
    /// did-you-mean suggestions.
    pub fn all() -> &'static [Field] {
        use Field::*;
        &[
            Artist,
            AlbumArtist,
            Album,
            Title,
            Genre,
            Composer,
            Year,
            Codec,
            Bitrate,
            SampleRate,
            BitDepth,
            Duration,
            Path,
            FileSize,
            Added,
            PlayCount,
            SkipCount,
            LastPlayed,
            Rating,
            Loved,
            TrackNo,
            DiscNo,
            Lossless,
            Cue,
        ]
    }

    pub fn is_text(self) -> bool {
        use Field::*;
        matches!(
            self,
            Artist | AlbumArtist | Album | Title | Genre | Composer | Codec | Path
        )
    }

    /// Does this field live in `track_stat` rather than `track`?
    pub fn needs_stats(self) -> bool {
        use Field::*;
        matches!(self, PlayCount | SkipCount | LastPlayed | Rating | Loved)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    Contains,
    NotContains,
}

impl Op {
    pub fn symbol(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "!=",
            Op::Gt => ">",
            Op::Lt => "<",
            Op::Ge => ">=",
            Op::Le => "<=",
            Op::Contains => "~",
            Op::NotContains => "!~",
        }
    }

    /// The operator meaning the opposite, where one exists. Used to push a
    /// `not` down into a predicate so the rule builder can still show it.
    pub fn negated(self) -> Option<Op> {
        Some(match self {
            Op::Eq => Op::Ne,
            Op::Ne => Op::Eq,
            Op::Gt => Op::Le,
            Op::Le => Op::Gt,
            Op::Lt => Op::Ge,
            Op::Ge => Op::Lt,
            Op::Contains => Op::NotContains,
            Op::NotContains => Op::Contains,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Num(f64),
    /// Seconds.
    Duration(f64),
    /// Days before now.
    RelativeDays(f64),
    Bool(bool),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Text(s) => {
                if s.chars().any(|c| c.is_whitespace() || c == '"') {
                    write!(f, "{s:?}")
                } else {
                    f.write_str(s)
                }
            }
            Value::Num(n) => {
                if n.fract() == 0.0 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{n}")
                }
            }
            Value::Duration(s) => write!(f, "{}s", *s as i64),
            Value::RelativeDays(d) => write!(f, "{}d", *d as i64),
            Value::Bool(b) => write!(f, "{b}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Predicate {
    pub field: Field,
    pub op: Op,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// An empty query matches everything.
    All,
    Pred(Predicate),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Field(Field),
    Random,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub filter: Expr,
    pub sort: Vec<Sort>,
    pub limit: Option<u32>,
    /// `limit N per artist`
    pub limit_per: Option<Field>,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            filter: Expr::All,
            sort: Vec::new(),
            limit: None,
            limit_per: None,
        }
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}",
            self.field.canonical(),
            self.op.symbol(),
            self.value
        )
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::All => Ok(()),
            Expr::Pred(p) => write!(f, "{p}"),
            Expr::Not(e) => write!(f, "not ({e})"),
            Expr::And(parts) => {
                let joined: Vec<String> = parts.iter().map(render_operand).collect();
                f.write_str(&joined.join(" and "))
            }
            Expr::Or(parts) => {
                let joined: Vec<String> = parts.iter().map(render_operand).collect();
                f.write_str(&joined.join(" or "))
            }
        }
    }
}

/// Parenthesise only where precedence would otherwise change the meaning.
fn render_operand(e: &Expr) -> String {
    match e {
        Expr::And(_) | Expr::Or(_) => format!("({e})"),
        _ => e.to_string(),
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        let filter = self.filter.to_string();
        if !filter.is_empty() {
            parts.push(filter);
        }
        if !self.sort.is_empty() {
            let keys: Vec<String> = self
                .sort
                .iter()
                .map(|s| {
                    let k = match s.key {
                        SortKey::Field(fd) => fd.canonical(),
                        SortKey::Random => "random",
                    };
                    if s.descending {
                        format!("{k} desc")
                    } else {
                        k.to_string()
                    }
                })
                .collect();
            parts.push(format!("sort {}", keys.join(", ")));
        }
        if let Some(n) = self.limit {
            match self.limit_per {
                Some(fd) => parts.push(format!("limit {n} per {}", fd.canonical())),
                None => parts.push(format!("limit {n}")),
            }
        }
        f.write_str(&parts.join(" "))
    }
}
