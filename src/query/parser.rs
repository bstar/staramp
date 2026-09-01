//! Hand-written lexer and recursive-descent parser.
//!
//! Not a combinator library, for four reasons that all matter here: every token
//! needs a byte span so the editor can highlight and point a caret; a
//! half-typed query must still yield a usable partial AST so the live match
//! count keeps working on every keystroke; `unknown field 'nad' — did you mean
//! 'and'?` needs edit-distance against the field table at the failure point;
//! and the grammar is small enough that the parser is shorter than the
//! integration would be.

use std::fmt;

use super::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: (usize, usize),
    pub hint: Option<String>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(h) = &self.hint {
            write!(f, " — {h}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Op(Op),
    LParen,
    RParen,
    Comma,
}

#[derive(Debug, Clone)]
struct Spanned {
    tok: Tok,
    span: (usize, usize),
}

fn lex(input: &str) -> Result<Vec<Spanned>, ParseError> {
    let b: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;

    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;

        match c {
            '(' => {
                out.push(Spanned {
                    tok: Tok::LParen,
                    span: (start, i + 1),
                });
                i += 1;
            }
            ')' => {
                out.push(Spanned {
                    tok: Tok::RParen,
                    span: (start, i + 1),
                });
                i += 1;
            }
            ',' => {
                out.push(Spanned {
                    tok: Tok::Comma,
                    span: (start, i + 1),
                });
                i += 1;
            }
            '"' | '\'' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                while i < b.len() && b[i] != quote {
                    // Backslash escapes, so a title can contain a quote.
                    if b[i] == '\\' && i + 1 < b.len() {
                        i += 1;
                    }
                    s.push(b[i]);
                    i += 1;
                }
                if i >= b.len() {
                    // Unterminated: take what there is. A half-typed string is
                    // the normal state while someone is still typing.
                    out.push(Spanned {
                        tok: Tok::Str(s),
                        span: (start, i),
                    });
                } else {
                    i += 1;
                    out.push(Spanned {
                        tok: Tok::Str(s),
                        span: (start, i),
                    });
                }
            }
            '=' | '!' | '<' | '>' | '~' | ':' => {
                let (op, len) = match (c, b.get(i + 1)) {
                    ('!', Some('=')) => (Op::Ne, 2),
                    ('!', Some('~')) => (Op::NotContains, 2),
                    ('>', Some('=')) => (Op::Ge, 2),
                    ('<', Some('=')) => (Op::Le, 2),
                    ('<', Some('>')) => (Op::Ne, 2),
                    ('=', Some('=')) => (Op::Eq, 2),
                    ('=', _) => (Op::Eq, 1),
                    ('>', _) => (Op::Gt, 1),
                    ('<', _) => (Op::Lt, 1),
                    ('~', _) => (Op::Contains, 1),
                    (':', _) => (Op::Contains, 1),
                    ('!', _) => {
                        return Err(ParseError {
                            message: "stray `!`".into(),
                            span: (start, i + 1),
                            hint: Some("did you mean `!=` or `!~`?".into()),
                        })
                    }
                    _ => unreachable!(),
                };
                i += len;
                out.push(Spanned {
                    tok: Tok::Op(op),
                    span: (start, i),
                });
            }
            _ => {
                while i < b.len() && !b[i].is_whitespace() {
                    let ch = b[i];
                    // `:` is an operator alias for `~`, but it is also the
                    // separator in a `3:45` duration. Keep it inside the word
                    // when it sits between two digits, or `duration > 3:45`
                    // lexes as three tokens and stops being a duration at all.
                    if ch == ':'
                        && i > start
                        && b[i - 1].is_ascii_digit()
                        && b.get(i + 1).is_some_and(|c| c.is_ascii_digit())
                    {
                        i += 1;
                        continue;
                    }
                    if "()=!<>~:,\"'".contains(ch) {
                        break;
                    }
                    i += 1;
                }
                let word: String = b[start..i].iter().collect();
                out.push(Spanned {
                    tok: Tok::Word(word),
                    span: (start, i),
                });
            }
        }
    }
    Ok(out)
}

pub fn parse(input: &str) -> Result<Query, ParseError> {
    let toks = lex(input)?;
    let mut p = Parser {
        toks,
        pos: 0,
        src_len: input.len(),
    };
    p.query()
}

struct Parser {
    toks: Vec<Spanned>,
    pos: usize,
    src_len: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|s| &s.tok)
    }

    fn peek_word(&self) -> Option<String> {
        match self.peek() {
            Some(Tok::Word(w)) => Some(w.to_ascii_lowercase()),
            _ => None,
        }
    }

    fn span(&self) -> (usize, usize) {
        self.toks
            .get(self.pos)
            .map(|s| s.span)
            .unwrap_or((self.src_len, self.src_len))
    }

    fn bump(&mut self) -> Option<Spanned> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.peek_word().as_deref() == Some(w) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn query(&mut self) -> Result<Query, ParseError> {
        let mut q = Query::default();

        if self.peek().is_some() && !self.at_clause_keyword() {
            q.filter = self.or_expr()?;
        }

        loop {
            if self.eat_word("sort") || self.eat_word("order") {
                q.sort = self.sort_keys()?;
            } else if self.eat_word("limit") {
                let (n, span) = self.number()?;
                if n < 0.0 {
                    return Err(ParseError {
                        message: "limit must not be negative".into(),
                        span,
                        hint: None,
                    });
                }
                q.limit = Some(n as u32);
                if self.eat_word("per") {
                    q.limit_per = Some(self.field()?);
                }
            } else {
                break;
            }
        }

        if let Some(rest) = self.toks.get(self.pos) {
            return Err(ParseError {
                message: format!("unexpected `{}`", describe(&rest.tok)),
                span: rest.span,
                hint: Some("expected `sort` or `limit` here".into()),
            });
        }
        Ok(q)
    }

    fn at_clause_keyword(&self) -> bool {
        matches!(
            self.peek_word().as_deref(),
            Some("sort") | Some("order") | Some("limit")
        )
    }

    fn sort_keys(&mut self) -> Result<Vec<Sort>, ParseError> {
        let mut keys = Vec::new();
        loop {
            let key = if self.eat_word("random") {
                SortKey::Random
            } else {
                SortKey::Field(self.field()?)
            };
            let descending = if self.eat_word("desc") {
                true
            } else {
                self.eat_word("asc");
                false
            };
            keys.push(Sort { key, descending });
            if !matches!(self.peek(), Some(Tok::Comma)) {
                break;
            }
            self.pos += 1;
        }
        Ok(keys)
    }

    fn field(&mut self) -> Result<Field, ParseError> {
        let span = self.span();
        match self.bump() {
            Some(Spanned {
                tok: Tok::Word(w), ..
            }) => Field::parse(&w).ok_or_else(|| ParseError {
                message: format!("unknown field `{w}`"),
                span,
                hint: suggest(&w),
            }),
            _ => Err(ParseError {
                message: "expected a field name".into(),
                span,
                hint: None,
            }),
        }
    }

    fn number(&mut self) -> Result<(f64, (usize, usize)), ParseError> {
        let span = self.span();
        match self.bump() {
            Some(Spanned {
                tok: Tok::Word(w), ..
            }) => w.parse().map(|n| (n, span)).map_err(|_| ParseError {
                message: format!("expected a number, got `{w}`"),
                span,
                hint: None,
            }),
            _ => Err(ParseError {
                message: "expected a number".into(),
                span,
                hint: None,
            }),
        }
    }

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut parts = vec![self.and_expr()?];
        while self.eat_word("or") {
            parts.push(self.and_expr()?);
        }
        Ok(if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            Expr::Or(parts)
        })
    }

    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut parts = vec![self.not_expr()?];
        loop {
            // `and` is optional: two predicates in a row imply it, so
            // `year >= 2015 codec = flac` means the same as spelling it out.
            let explicit = self.eat_word("and");
            if !explicit && !self.starts_operand() {
                break;
            }
            parts.push(self.not_expr()?);
        }
        Ok(if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            Expr::And(parts)
        })
    }

    fn starts_operand(&self) -> bool {
        match self.peek() {
            Some(Tok::LParen) => true,
            Some(Tok::Word(w)) => {
                let lw = w.to_ascii_lowercase();
                !matches!(
                    lw.as_str(),
                    "or" | "and" | "sort" | "order" | "limit" | "per" | "asc" | "desc"
                )
            }
            _ => false,
        }
    }

    fn not_expr(&mut self) -> Result<Expr, ParseError> {
        if self.eat_word("not") {
            return Ok(Expr::Not(Box::new(self.not_expr()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            let inner = self.or_expr()?;
            if matches!(self.peek(), Some(Tok::RParen)) {
                self.pos += 1;
            }
            // An unclosed paren is normal mid-typing; take what we have.
            return Ok(inner);
        }

        // Bare flags, so `loved` and `never` work without an operator.
        if let Some(w) = self.peek_word() {
            let flag = match w.as_str() {
                "loved" => Some(Predicate {
                    field: Field::Loved,
                    op: Op::Eq,
                    value: Value::Bool(true),
                }),
                "unloved" => Some(Predicate {
                    field: Field::Loved,
                    op: Op::Eq,
                    value: Value::Bool(false),
                }),
                "never" | "unplayed" => Some(Predicate {
                    field: Field::PlayCount,
                    op: Op::Eq,
                    value: Value::Num(0.0),
                }),
                "lossless" => Some(Predicate {
                    field: Field::Lossless,
                    op: Op::Eq,
                    value: Value::Bool(true),
                }),
                "cue" => Some(Predicate {
                    field: Field::Cue,
                    op: Op::Eq,
                    value: Value::Bool(true),
                }),
                _ => None,
            };
            if let Some(p) = flag {
                // Only a flag if no operator follows it.
                let is_flag = !matches!(
                    self.toks.get(self.pos + 1).map(|s| &s.tok),
                    Some(Tok::Op(_))
                );
                if is_flag {
                    self.pos += 1;
                    return Ok(Expr::Pred(p));
                }
            }
        }

        let field = self.field()?;
        let op_span = self.span();
        let op = match self.bump() {
            Some(Spanned {
                tok: Tok::Op(o), ..
            }) => o,
            other => {
                return Err(ParseError {
                    message: match other {
                        Some(s) => format!(
                            "expected an operator after `{}`, got `{}`",
                            field.canonical(),
                            describe(&s.tok)
                        ),
                        None => format!("expected an operator after `{}`", field.canonical()),
                    },
                    span: op_span,
                    hint: Some("try `=`, `~`, `>`, `<`".into()),
                })
            }
        };
        let value = self.value(field)?;
        Ok(Expr::Pred(Predicate { field, op, value }))
    }

    fn value(&mut self, field: Field) -> Result<Value, ParseError> {
        let span = self.span();
        let raw = match self.bump() {
            Some(Spanned {
                tok: Tok::Str(s), ..
            }) => return Ok(Value::Text(s)),
            Some(Spanned {
                tok: Tok::Word(w), ..
            }) => w,
            _ => {
                return Err(ParseError {
                    message: format!("expected a value for `{}`", field.canonical()),
                    span,
                    hint: None,
                })
            }
        };
        Ok(parse_value(&raw, field))
    }
}

/// Interpret a bare word as the most specific thing it can be.
fn parse_value(raw: &str, field: Field) -> Value {
    let lower = raw.to_ascii_lowercase();

    if field == Field::Loved || field == Field::Lossless || field == Field::Cue {
        return Value::Bool(matches!(lower.as_str(), "true" | "yes" | "1"));
    }

    // Relative dates: 90d, 12w, 6mo, 2y.
    if let Some(days) = parse_relative_days(&lower) {
        return Value::RelativeDays(days);
    }
    match lower.as_str() {
        "today" => return Value::RelativeDays(0.0),
        "yesterday" => return Value::RelativeDays(1.0),
        "thisweek" => return Value::RelativeDays(7.0),
        "thismonth" => return Value::RelativeDays(30.0),
        "thisyear" => return Value::RelativeDays(365.0),
        _ => {}
    }

    // Durations: 3:45, 5m, 90s.
    if field == Field::Duration {
        if let Some((m, s)) = lower.split_once(':') {
            if let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) {
                return Value::Duration(m * 60.0 + s);
            }
        }
        if let Some(n) = lower.strip_suffix('m').and_then(|n| n.parse::<f64>().ok()) {
            return Value::Duration(n * 60.0);
        }
        if let Some(n) = lower.strip_suffix('s').and_then(|n| n.parse::<f64>().ok()) {
            return Value::Duration(n);
        }
        if let Ok(n) = lower.parse::<f64>() {
            return Value::Duration(n);
        }
    }

    // `320k` for bitrate, `44.1k` for sample rate.
    if let Some(n) = lower.strip_suffix('k').and_then(|n| n.parse::<f64>().ok()) {
        return Value::Num(n * 1000.0);
    }
    if let Ok(n) = lower.parse::<f64>() {
        return Value::Num(n);
    }
    Value::Text(raw.to_string())
}

fn parse_relative_days(s: &str) -> Option<f64> {
    for (suffix, mult) in [("mo", 30.0), ("d", 1.0), ("w", 7.0), ("y", 365.0)] {
        if let Some(head) = s.strip_suffix(suffix) {
            if head.is_empty() {
                continue;
            }
            if let Ok(n) = head.parse::<f64>() {
                return Some(n * mult);
            }
        }
    }
    None
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Word(w) => w.clone(),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Op(o) => o.symbol().to_string(),
        Tok::LParen => "(".into(),
        Tok::RParen => ")".into(),
        Tok::Comma => ",".into(),
    }
}

/// Nearest field or keyword, for a did-you-mean hint.
fn suggest(word: &str) -> Option<String> {
    let mut candidates: Vec<&str> = Field::all().iter().map(|f| f.canonical()).collect();
    candidates.extend(["and", "or", "not", "sort", "limit", "loved", "never"]);

    let w = word.to_ascii_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = edit_distance(&w, c);
        if best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, c));
        }
    }
    match best {
        // Only suggest when it is actually close; a wild guess is noise.
        Some((d, c)) if d <= 2 => Some(format!("did you mean `{c}`?")),
        _ => None,
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Query {
        parse(s).unwrap_or_else(|e| panic!("{s:?} failed: {e}"))
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert_eq!(parse("").unwrap().filter, Expr::All);
        assert_eq!(parse("   ").unwrap().filter, Expr::All);
    }

    #[test]
    fn parses_a_single_predicate() {
        let p = q("year >= 2015");
        assert_eq!(
            p.filter,
            Expr::Pred(Predicate {
                field: Field::Year,
                op: Op::Ge,
                value: Value::Num(2015.0)
            })
        );
    }

    #[test]
    fn quoted_values_keep_their_spaces() {
        let p = q(r#"genre ~ "power metal""#);
        assert_eq!(
            p.filter,
            Expr::Pred(Predicate {
                field: Field::Genre,
                op: Op::Contains,
                value: Value::Text("power metal".into())
            })
        );
    }

    #[test]
    fn juxtaposition_means_and() {
        let a = q("year >= 2015 codec = flac");
        let b = q("year >= 2015 and codec = flac");
        assert_eq!(a.filter, b.filter);
    }

    #[test]
    fn and_binds_tighter_than_or() {
        let p = q("year = 1 and rating = 2 or track = 3");
        match p.filter {
            Expr::Or(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(parts[0], Expr::And(_)));
            }
            other => panic!("expected a top-level or, got {other:?}"),
        }
    }

    #[test]
    fn parentheses_override_precedence() {
        let p = q("artist = x and (year = 1 or year = 2)");
        match p.filter {
            Expr::And(parts) => assert!(matches!(parts[1], Expr::Or(_))),
            other => panic!("expected and, got {other:?}"),
        }
    }

    #[test]
    fn the_headline_query_from_the_plan_parses() {
        let p = q(r#"genre ~ "power metal" and year >= 2015 and codec = flac
                     and playcount < 3 and added > 90d
                     sort added desc limit 200"#);
        assert!(matches!(p.filter, Expr::And(_)));
        assert_eq!(p.limit, Some(200));
        assert_eq!(p.sort.len(), 1);
        assert!(p.sort[0].descending);
    }

    #[test]
    fn relative_dates_become_days() {
        assert_eq!(
            q("added > 90d").filter,
            Expr::Pred(Predicate {
                field: Field::Added,
                op: Op::Gt,
                value: Value::RelativeDays(90.0)
            })
        );
        assert_eq!(
            q("added > 2w").filter,
            Expr::Pred(Predicate {
                field: Field::Added,
                op: Op::Gt,
                value: Value::RelativeDays(14.0)
            })
        );
        assert_eq!(
            q("added > 6mo").filter,
            Expr::Pred(Predicate {
                field: Field::Added,
                op: Op::Gt,
                value: Value::RelativeDays(180.0)
            })
        );
    }

    #[test]
    fn durations_accept_several_spellings() {
        for (src, want) in [
            ("duration > 3:45", 225.0),
            ("duration > 5m", 300.0),
            ("duration > 90s", 90.0),
            ("duration > 90", 90.0),
        ] {
            match q(src).filter {
                Expr::Pred(p) => assert_eq!(p.value, Value::Duration(want), "{src}"),
                _ => panic!("{src}"),
            }
        }
    }

    #[test]
    fn bare_flags_work_without_an_operator() {
        assert_eq!(
            q("loved").filter,
            Expr::Pred(Predicate {
                field: Field::Loved,
                op: Op::Eq,
                value: Value::Bool(true)
            })
        );
        assert_eq!(
            q("never").filter,
            Expr::Pred(Predicate {
                field: Field::PlayCount,
                op: Op::Eq,
                value: Value::Num(0.0)
            })
        );
    }

    #[test]
    fn field_aliases_resolve_to_the_canonical_field() {
        for alias in ["artist", "ar", "a"] {
            match q(&format!("{alias} = x")).filter {
                Expr::Pred(p) => assert_eq!(p.field, Field::Artist),
                _ => panic!(),
            }
        }
    }

    #[test]
    fn an_unknown_field_suggests_a_real_one() {
        let e = parse("nad year >= 1990").unwrap_err();
        assert!(e.message.contains("unknown field"), "{}", e.message);
        let hint = e.hint.expect("expected a suggestion");
        assert!(hint.contains("and"), "unhelpful hint: {hint}");
    }

    #[test]
    fn errors_carry_a_span_that_points_at_the_problem() {
        let src = "year >= 2015 and nonsuchfield = 3";
        let e = parse(src).unwrap_err();
        let (s, en) = e.span;
        assert_eq!(&src[s..en], "nonsuchfield");
    }

    #[test]
    fn queries_round_trip_through_their_own_rendering() {
        for src in [
            "artist = Angra",
            r#"genre ~ "power metal" and year >= 2015"#,
            "artist = x or artist = y",
            "not (artist = x)",
            "year >= 2015 sort added desc limit 200",
            "loved and codec = flac sort random limit 50",
        ] {
            let a = q(src);
            let rendered = a.to_string();
            let b =
                parse(&rendered).unwrap_or_else(|e| panic!("re-parse of {rendered:?} failed: {e}"));
            assert_eq!(a, b, "{src:?} -> {rendered:?}");
        }
    }

    #[test]
    fn a_half_typed_query_still_parses_as_far_as_it_got() {
        // The live match count runs on every keystroke, so this is the normal
        // state rather than an edge case.
        assert!(parse(r#"genre ~ "met"#).is_ok());
        assert!(parse("year >= 2015 and (codec = flac").is_ok());
    }

    #[test]
    fn sort_accepts_multiple_keys() {
        let p = q("sort artist, year desc, random");
        assert_eq!(p.sort.len(), 3);
        assert_eq!(p.sort[0].key, SortKey::Field(Field::Artist));
        assert!(p.sort[1].descending);
        assert_eq!(p.sort[2].key, SortKey::Random);
    }

    #[test]
    fn limit_per_field_parses() {
        let p = q("loved limit 3 per artist");
        assert_eq!(p.limit, Some(3));
        assert_eq!(p.limit_per, Some(Field::Artist));
    }
}
