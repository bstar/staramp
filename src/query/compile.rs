//! Compiling a query to parameterised SQL.
//!
//! Values are always bound, never interpolated: playlist names and search text
//! reach this path, so string-building would be an injection waiting to happen.
//! Joins are lazy — a query that never mentions a stats field does not pay for
//! the join.

use super::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Int(i64),
    Real(f64),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Param>,
    pub needs_stats: bool,
}

/// `now` is passed in rather than read from the clock so tests are
/// deterministic and a saved smart playlist re-evaluates against the moment it
/// is opened.
pub fn compile(q: &Query, now_epoch: i64, count_only: bool) -> Compiled {
    let mut params = Vec::new();
    let needs_stats = expr_needs_stats(&q.filter)
        || q.sort.iter().any(|s| match s.key {
            SortKey::Field(f) => f.needs_stats(),
            SortKey::Random => false,
        });

    let where_sql = compile_expr(&q.filter, now_epoch, &mut params);

    let select = if count_only {
        "SELECT COUNT(*)"
    } else {
        "SELECT t.id"
    };
    let join = if needs_stats {
        " LEFT JOIN activity.track_stat s ON s.uri = t.uri"
    } else {
        ""
    };

    let mut sql = format!("{select} FROM track t{join} WHERE t.hidden = 0 AND ({where_sql})");

    if !count_only {
        if !q.sort.is_empty() {
            let keys: Vec<String> = q
                .sort
                .iter()
                .map(|s| {
                    let col = match s.key {
                        SortKey::Field(f) => column(f).to_string(),
                        SortKey::Random => "random()".to_string(),
                    };
                    if s.descending {
                        format!("{col} DESC")
                    } else {
                        col
                    }
                })
                .collect();
            sql.push_str(&format!(" ORDER BY {}", keys.join(", ")));
        }
        if let Some(n) = q.limit {
            sql.push_str(" LIMIT ?");
            params.push(Param::Int(n as i64));
        }
    }

    Compiled {
        sql,
        params,
        needs_stats,
    }
}

fn expr_needs_stats(e: &Expr) -> bool {
    match e {
        Expr::All => false,
        Expr::Pred(p) => p.field.needs_stats(),
        Expr::Not(inner) => expr_needs_stats(inner),
        Expr::And(parts) | Expr::Or(parts) => parts.iter().any(expr_needs_stats),
    }
}

fn compile_expr(e: &Expr, now: i64, params: &mut Vec<Param>) -> String {
    match e {
        Expr::All => "1".into(),
        Expr::Pred(p) => compile_pred(p, now, params),
        Expr::Not(inner) => format!("NOT ({})", compile_expr(inner, now, params)),
        Expr::And(parts) => join_parts(parts, " AND ", now, params),
        Expr::Or(parts) => join_parts(parts, " OR ", now, params),
    }
}

fn join_parts(parts: &[Expr], sep: &str, now: i64, params: &mut Vec<Param>) -> String {
    if parts.is_empty() {
        return "1".into();
    }
    let rendered: Vec<String> = parts
        .iter()
        .map(|p| format!("({})", compile_expr(p, now, params)))
        .collect();
    rendered.join(sep)
}

/// The SQL expression for a field.
///
/// Stats columns are wrapped in COALESCE because the LEFT JOIN yields NULL for
/// a track that has never been played, and `NULL = 0` is NULL, not true. Without
/// this the entire "never played" feature silently returns nothing.
fn column(f: Field) -> &'static str {
    use Field::*;
    match f {
        Artist => "t.artist",
        AlbumArtist => "t.album_artist",
        Album => "t.album",
        Title => "t.title",
        Genre => "t.genre",
        Composer => "t.composer",
        Year => "t.year",
        Codec => "t.codec",
        Bitrate => "t.bitrate_kbps",
        SampleRate => "t.sample_rate",
        BitDepth => "t.bit_depth",
        Duration => "t.duration_ms",
        Path => "t.uri",
        FileSize => "t.file_size",
        Added => "t.added_at",
        TrackNo => "t.track_no",
        DiscNo => "t.disc_no",
        Lossless => "t.is_lossless",
        Cue => "(t.cue_file_id IS NOT NULL)",
        PlayCount => "COALESCE(s.play_count, 0)",
        SkipCount => "COALESCE(s.skip_count, 0)",
        LastPlayed => "COALESCE(s.last_played_at, 0)",
        Rating => "COALESCE(s.rating, 0)",
        Loved => "COALESCE(s.loved, 0)",
    }
}

fn compile_pred(p: &Predicate, now: i64, params: &mut Vec<Param>) -> String {
    let col = column(p.field);

    // Text comparisons are case-insensitive; nobody types exact case.
    if p.field.is_text() {
        return match (&p.value, p.op) {
            (Value::Text(s), Op::Contains) => {
                params.push(Param::Text(format!("%{}%", escape_like(s))));
                format!("{col} LIKE ? ESCAPE '\\'")
            }
            (Value::Text(s), Op::NotContains) => {
                params.push(Param::Text(format!("%{}%", escape_like(s))));
                format!("({col} IS NULL OR {col} NOT LIKE ? ESCAPE '\\')")
            }
            (Value::Text(s), Op::Ne) => {
                params.push(Param::Text(s.clone()));
                format!("({col} IS NULL OR {col} <> ? COLLATE NOCASE)")
            }
            (Value::Text(s), _) => {
                params.push(Param::Text(s.clone()));
                format!("{col} = ? COLLATE NOCASE")
            }
            (other, op) => {
                params.push(Param::Text(other.to_string()));
                format!("{col} {} ?", sql_op(op))
            }
        };
    }

    let numeric = match &p.value {
        Value::Num(n) => *n,
        // Stored in milliseconds.
        Value::Duration(s) => s * 1000.0,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        // `added > 90d` means "added since 90 days ago", so a *newer* timestamp.
        // The comparison direction is preserved by converting the value, not by
        // flipping the operator.
        Value::RelativeDays(d) => (now - (d * 86_400.0) as i64) as f64,
        Value::Text(t) => {
            params.push(Param::Text(t.clone()));
            return format!("{col} {} ?", sql_op(p.op));
        }
    };

    if numeric.fract() == 0.0 {
        params.push(Param::Int(numeric as i64));
    } else {
        params.push(Param::Real(numeric));
    }
    format!("{col} {} ?", sql_op(p.op))
}

fn sql_op(op: Op) -> &'static str {
    match op {
        Op::Eq => "=",
        Op::Ne => "<>",
        Op::Gt => ">",
        Op::Lt => "<",
        Op::Ge => ">=",
        Op::Le => "<=",
        Op::Contains => "LIKE",
        Op::NotContains => "NOT LIKE",
    }
}

/// Escape LIKE wildcards so a search for `100%` does not match everything.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::parse;

    const NOW: i64 = 1_700_000_000;

    fn c(src: &str) -> Compiled {
        compile(&parse(src).unwrap(), NOW, false)
    }

    #[test]
    fn values_are_bound_never_interpolated() {
        let q = c(r#"artist = "Robert'); DROP TABLE track;--""#);
        assert!(
            !q.sql.contains("DROP"),
            "the value leaked into the SQL: {}",
            q.sql
        );
        assert_eq!(q.params.len(), 1);
    }

    #[test]
    fn a_query_without_stats_fields_does_not_join_stats() {
        let q = c("artist = Angra and year > 2000");
        assert!(!q.needs_stats);
        assert!(!q.sql.contains("track_stat"), "{}", q.sql);
    }

    #[test]
    fn a_query_using_stats_joins_them() {
        let q = c("playcount > 5");
        assert!(q.needs_stats);
        assert!(q.sql.contains("LEFT JOIN activity.track_stat"), "{}", q.sql);
    }

    #[test]
    fn never_played_survives_the_left_join() {
        // The whole "deep cuts" feature depends on this: without COALESCE the
        // join yields NULL for unplayed tracks and `NULL = 0` is NULL.
        let q = c("playcount = 0");
        assert!(q.sql.contains("COALESCE(s.play_count, 0)"), "{}", q.sql);
    }

    #[test]
    fn text_matching_is_case_insensitive_and_uses_like_for_contains() {
        let eq = c("artist = angra");
        assert!(eq.sql.contains("COLLATE NOCASE"), "{}", eq.sql);

        let ct = c(r#"genre ~ "power metal""#);
        assert!(ct.sql.contains("LIKE"), "{}", ct.sql);
        assert_eq!(ct.params[0], Param::Text("%power metal%".into()));
    }

    #[test]
    fn like_wildcards_in_user_input_are_escaped() {
        let q = c(r#"title ~ "100%""#);
        assert_eq!(q.params[0], Param::Text("%100\\%%".into()));
    }

    #[test]
    fn relative_dates_become_an_absolute_timestamp() {
        let q = c("added > 90d");
        let want = NOW - 90 * 86_400;
        assert_eq!(q.params[0], Param::Int(want));
        // Direction is preserved: "added in the last 90 days" is added_at > T.
        assert!(q.sql.contains("t.added_at >"), "{}", q.sql);
    }

    #[test]
    fn durations_are_compared_in_milliseconds() {
        let q = c("duration > 5m");
        assert_eq!(q.params[0], Param::Int(300_000));
    }

    #[test]
    fn hidden_tracks_are_always_excluded() {
        // A disc-image cue hides its backing file; smart playlists must not
        // resurrect one 70-minute track alongside its own thirteen.
        assert!(c("loved").sql.contains("t.hidden = 0"));
        assert!(c("").sql.contains("t.hidden = 0"));
    }

    #[test]
    fn sort_and_limit_are_emitted() {
        let q = c("loved sort added desc limit 200");
        assert!(q.sql.contains("ORDER BY t.added_at DESC"), "{}", q.sql);
        assert!(q.sql.ends_with("LIMIT ?"), "{}", q.sql);
        assert_eq!(*q.params.last().unwrap(), Param::Int(200));
    }

    #[test]
    fn a_count_query_drops_ordering_and_limit() {
        let q = compile(
            &parse("loved sort added desc limit 200").unwrap(),
            NOW,
            true,
        );
        assert!(q.sql.starts_with("SELECT COUNT(*)"), "{}", q.sql);
        assert!(!q.sql.contains("ORDER BY"), "{}", q.sql);
        assert!(!q.sql.contains("LIMIT"), "{}", q.sql);
    }

    #[test]
    fn boolean_flags_compile_to_one_and_zero() {
        assert_eq!(c("loved").params[0], Param::Int(1));
        assert_eq!(c("unloved").params[0], Param::Int(0));
    }

    #[test]
    fn nested_boolean_structure_is_preserved() {
        let q = c("artist = x and (year = 1 or year = 2)");
        assert!(q.sql.contains(" OR "), "{}", q.sql);
        assert!(q.sql.contains(" AND "), "{}", q.sql);
        assert_eq!(q.params.len(), 3);
    }

    #[test]
    fn not_equals_on_text_also_matches_rows_where_the_field_is_null() {
        // `artist != x` should include tracks with no artist at all; plain
        // `<>` against NULL is NULL and silently drops them.
        let q = c("artist != Angra");
        assert!(q.sql.contains("IS NULL OR"), "{}", q.sql);
    }
}
