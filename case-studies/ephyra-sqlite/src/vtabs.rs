// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Virtual tables of the case's database: R-Tree (`rtree`, `rtree_i32`),
//! Geopoly and FTS5 tables (and `fts5vocab` tables over an FTS5 one),
//! `vt{k}`, [`crate::model::Model::vtabs`]`[k]`, all in main. The model
//! keeps each one's rows by rowid exactly as SQLite stores them, so that
//! every statement drawn is one SQLite runs:
//!
//! - an R-Tree's coordinates are drawn exactly representable as 32-bit
//!   floats (integers and halves of magnitude below 2^20; integers for
//!   `rtree_i32`), so the stored values are the inserted ones, as REAL
//!   (INTEGER for `rtree_i32`); each minimum at most its maximum (else
//!   "rtree constraint failed"), ids unique unless `OR REPLACE`;
//! - a Geopoly table's `_shape` is a valid polygon (a JSON text of a
//!   closed ring or `geopoly_regular(..)`), which the model keeps as the
//!   SQL that made it (NULL once `geopoly_xform` moved it): nothing reads
//!   it but SQLite;
//! - an FTS5 table's columns hold the inserted values (texts of a small
//!   vocabulary, NULLs, integers); its MATCH queries are of the syntax its
//!   `detail` takes (no phrase of more than one token, NEAR or `^` without
//!   `detail=full`, no column filter with `detail=none`; a trigram table
//!   only `detail=full`, where a word is a phrase of its trigrams).
//!
//! Writes go by rowid (a range or one), never by MATCH, which only a model
//! of tokenization could follow. Queries read them through a CTE body
//! ([`VQuery`]), which an outer query joins with tables; SQLite flattens
//! it, so the outer constraints reach the virtual table's `xBestIndex`.

use ephyra::Source;

use crate::{
    model::Value,
    statements::literal,
};

/// The most virtual tables a case makes.
pub const MAX_VTABS: usize = 4;

/// The most rows a virtual table holds.
pub const MAX_ROWS: usize = 400;

/// An FTS5 tokenizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Unicode61,
    Diacritics,
    Ascii,
    Porter,
    PorterAscii,
    Trigram,
    TrigramCase,
    /// `unicode61 tokenchars '-.' separators 'x'`: hyphens and dots
    /// inside tokens, `x` a separator.
    Tokenchars,
    /// `unicode61 categories 'L* N* ..' remove_diacritics 1`: the
    /// letters and numbers and the categories of [`CATEGORIES`] the mask
    /// sets.
    Categories(u64),
    /// `ascii separators '-'`.
    AsciiSeparators,
    /// `porter unicode61 remove_diacritics 1`.
    PorterDiacritics,
    /// `trigram remove_diacritics 1`.
    TrigramDiacritics,
}

impl Tok {
    pub const ALL: [Tok; 12] = [Tok::Unicode61, Tok::Diacritics, Tok::Ascii, Tok::Porter, Tok::PorterAscii, Tok::Trigram, Tok::TrigramCase, Tok::Tokenchars, Tok::Categories(0), Tok::AsciiSeparators, Tok::PorterDiacritics, Tok::TrigramDiacritics];

    fn sql(self) -> Option<String> {
        if let Tok::Categories(m) = self {
            let more: String = CATEGORIES.iter().enumerate().filter(|(k, _)| m >> k & 1 != 0).map(|(_, c)| format!(" {c}")).collect();
            return Some(format!("unicode61 categories ''L* N*{more}'' remove_diacritics 1"));
        }
        let s = match self {
            Tok::Unicode61 => None,
            Tok::Diacritics => Some("unicode61 remove_diacritics 2"),
            Tok::Ascii => Some("ascii"),
            Tok::Porter => Some("porter"),
            Tok::PorterAscii => Some("porter ascii"),
            Tok::Trigram => Some("trigram"),
            Tok::TrigramCase => Some("trigram case_sensitive 1"),
            Tok::Tokenchars => Some("unicode61 tokenchars ''-.'' separators ''x''"),
            Tok::AsciiSeparators => Some("ascii separators ''-''"),
            Tok::PorterDiacritics => Some("porter unicode61 remove_diacritics 1"),
            Tok::TrigramDiacritics => Some("trigram remove_diacritics 1"),
            Tok::Categories(_) => None,
        };
        s.map(String::from)
    }

    /// The tokenizer as `sql` spells it, or (by `k`, 0 to 3) in an
    /// equivalent or near form: its default arguments spelled out, its
    /// arguments quoted (`fts5ConfigSkipLiteral`), more `tokenchars` and
    /// `separators`, a Porter tokenizer around `unicode61` or `ascii`
    /// named, `remove_diacritics` 0 to 2. Nothing models the tokens.
    fn spelled(self, k: u8) -> Option<String> {
        let more = match (self, k) {
            (_, 0) => None,
            (Tok::Unicode61, 1) => Some("unicode61"),
            (Tok::Unicode61, 2) => Some("unicode61 remove_diacritics 0"),
            (Tok::Unicode61, _) => Some("unicode61 separators ''.,'' remove_diacritics 1"),
            (Tok::Diacritics, 1) => Some("unicode61 ''remove_diacritics'' ''2''"),
            (Tok::Diacritics, 2) => Some("unicode61 remove_diacritics 2 tokenchars ''_''"),
            (Tok::Ascii, 1) => Some("ascii tokenchars ''_-''"),
            (Tok::Ascii, 2) => Some("ascii separators ''.''"),
            (Tok::Porter, 1) => Some("porter unicode61"),
            (Tok::Porter, 2) => Some("porter ''unicode61'' remove_diacritics 2"),
            (Tok::Porter, _) => Some("porter unicode61 remove_diacritics 0 tokenchars ''_''"),
            (Tok::PorterAscii, 1) => Some("porter ascii separators ''x''"),
            (Tok::Trigram, 1) => Some("trigram case_sensitive 0"),
            (Tok::Trigram, 2) => Some("trigram remove_diacritics 0"),
            (Tok::Trigram, _) => Some("trigram ''case_sensitive'' 0 remove_diacritics 0"),
            (Tok::TrigramCase, 1) => Some("trigram remove_diacritics 0 case_sensitive 1"),
            (Tok::Tokenchars, 1) => Some("unicode61 tokenchars ''-.'' separators ''x'' remove_diacritics 0"),
            (Tok::Tokenchars, 2) => Some("unicode61 tokenchars ''-.'''''' separators ''x''"),
            (Tok::AsciiSeparators, 1) => Some("ascii separators ''-'' tokenchars ''_''"),
            (Tok::PorterDiacritics, 1) => Some("porter unicode61 remove_diacritics 2"),
            (Tok::PorterDiacritics, 2) => Some("porter unicode61 remove_diacritics 1 categories ''L* N* Co''"),
            (Tok::TrigramDiacritics, 1) => Some("trigram remove_diacritics 1 case_sensitive 0"),
            (Tok::Categories(_), 1) => return self.sql().map(|s| s.replace("remove_diacritics 1", "remove_diacritics 2")),
            _ => None,
        };
        more.map(String::from).or_else(|| self.sql())
    }

    fn trigram(self) -> bool {
        matches!(self, Tok::Trigram | Tok::TrigramCase | Tok::TrigramDiacritics)
    }
}

/// An FTS5 table's prefix indexes.
pub const PREFIXES: [&str; 5] = ["", "prefix='2 3'", "prefix=1", "prefix='1 2 3 4'", "prefix=2, prefix=4"];

/// [`Kind::Fts5`]'s `opts`.
pub const OPTS: [&str; 3] = ["tokendata=1", "locale=1", "contentless_unindexed=1"];

/// The Unicode categories a `categories` option may add to `L* N*`.
pub const CATEGORIES: [&str; 33] = [
    "Co", "Mn", "Mc", "Me", "Nd", "Pc", "Pd", "Pe", "Pf", "Pi", "Po", "Ps", "Sc", "Sk", "Sm", "So", "Zs", "Cc", "Cf", "P*", "S*", "Z*", "Lu", "Ll", "Lt", "Lm",
    "Lo", "Nl", "No", "M*", "Cs", "Zl", "Zp",
];

/// An FTS5 table's `detail` option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Full,
    Column,
    None,
}

/// What a virtual table is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `rtree(k, l0, h0, .., +p0, ..)` (`rtree_i32` with `int`): `dims`
    /// pairs of coordinates, `aux` auxiliary columns.
    Rtree { dims: usize, int: bool, aux: usize },
    /// `geopoly(p0, ..)`.
    Geopoly { aux: usize },
    /// `fts5(s0, .., [tokenize=..], [prefix=..], [detail=..],
    /// [columnsize=0])`.
    /// `contentless`: `content='', contentless_delete=1` (its columns
    /// read NULL; an UPDATE sets them all); `unindexed`: its last column
    /// `UNINDEXED` (of two or more); `prefix` one of [`PREFIXES`] (none
    /// for 0); `opts` a mask of [`OPTS`]: `tokendata=1`, `locale=1` (texts
    /// now and then inserted as `fts5_locale('..', text)`, which read back
    /// as the text), `contentless_unindexed=1` (of a contentless table
    /// with an UNINDEXED column, which reads back what was inserted).
    Fts5 { cols: usize, tok: Tok, prefix: u8, detail: Detail, columnsize: bool, contentless: bool, unindexed: bool, opts: u8 },
    /// `fts5vocab(vt{of}, 'row'|'col'|'instance')`.
    Vocab { of: usize, form: u8 },
}

/// A seed for an FTS5 table's spelling ([`Spelling`]): a hash of the
/// table and its number, so that a table is spelled one way.
fn spelling_seed(kind: &Kind, k: usize) -> u64 {
    format!("{kind:?}{k}").bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Choices of how an FTS5 table's CREATE spells its parts, taken from a
/// seed: option names in lower or upper case or the shortest prefix
/// `fts5ConfigParseSpecial` reads as the option (it takes the first
/// option a prefix of whose name the given one is), spaces around `=`,
/// values bare or quoted in any of `'`, `"`, `[]`, `` ` ``; column names
/// bare or quoted.
struct Spelling(u64);

impl Spelling {
    /// A choice of `n` (0 the plain spelling).
    fn take(&mut self, n: u64) -> u8 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % n) as u8
    }

    /// `w` bare (half the time) or quoted.
    fn word(&mut self, w: &str) -> String {
        match self.take(8) {
            0 => format!("\"{w}\""),
            1 => format!("[{w}]"),
            2 => format!("`{w}`"),
            3 => format!("'{w}'"),
            _ => w.to_string(),
        }
    }

    /// `name=value`; `quoted` a value that holds quotes doubled for `'`.
    fn option(&mut self, name: &str, value: &str, quoted: bool) -> String {
        let short = match name {
            "prefix" => "p",
            "tokenize" => "tok",
            "content" => "content",
            "contentless_delete" => "contentless_d",
            "contentless_unindexed" => "contentless_u",
            "columnsize" => "col",
            "locale" => "l",
            "detail" => "d",
            "tokendata" => "tokend",
            n => n,
        };
        let name = match self.take(6) {
            0 => name.to_uppercase(),
            1 => short.to_string(),
            2 => short.to_uppercase(),
            _ => name.to_string(),
        };
        let eq = if self.take(5) == 0 { " = " } else { "=" };
        let bare = !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        let value = if quoted {
            let raw = value.replace("''", "'");
            match self.take(3) {
                0 if !raw.contains('"') => format!("\"{raw}\""),
                1 if !raw.contains(']') => format!("[{raw}]"),
                _ => format!("'{value}'"),
            }
        } else {
            match self.take(6) {
                0 => format!("\"{value}\""),
                1 if !value.contains(']') => format!("[{value}]"),
                2 => format!("`{value}`"),
                3 if bare => value.to_string(),
                _ => format!("'{value}'"),
            }
        };
        format!("{name}{eq}{value}")
    }
}

impl Kind {
    /// The columns as `SELECT *` gives them (rowid aside).
    pub fn columns(&self) -> Vec<String> {
        match self {
            Kind::Rtree { dims, aux, .. } => std::iter::once("k".to_string()).chain((0..*dims).flat_map(|d| [format!("l{d}"), format!("h{d}")])).chain((0..*aux).map(|j| format!("p{j}"))).collect(),
            Kind::Geopoly { aux } => std::iter::once("_shape".to_string()).chain((0..*aux).map(|j| format!("p{j}"))).collect(),
            Kind::Fts5 { cols, .. } => (0..*cols).map(|j| format!("s{j}")).collect(),
            Kind::Vocab { form: 0, .. } => ["term", "doc", "cnt"].map(String::from).to_vec(),
            Kind::Vocab { form: 1, .. } => ["term", "col", "doc", "cnt"].map(String::from).to_vec(),
            Kind::Vocab { .. } => ["term", "doc", "col", "\"offset\""].map(String::from).to_vec(),
        }
    }

    fn create_sql(&self, k: usize) -> String {
        let args = match self {
            Kind::Rtree { int, .. } => format!("{}({})", if *int { "rtree_i32" } else { "rtree" }, self.columns().iter().enumerate().map(|(c, n)| if n.starts_with('p') && c > 0 { format!("+{n}") } else { n.clone() }).collect::<Vec<_>>().join(", ")),
            Kind::Geopoly { .. } => format!("geopoly({})", self.columns()[1..].join(", ")),
            Kind::Fts5 { cols, tok, prefix, detail, columnsize, contentless, unindexed, opts } => {
                // The spelling of every part a function of the table (no
                // draw): `sqlite3Fts5ConfigParse` and `fts5ConfigParseSpecial`
                // read each form, the tokenizers their arguments.
                let mut st = Spelling(spelling_seed(self, k));
                let mut a: Vec<String> = self.columns().into_iter().map(|c| st.word(&c)).collect();
                if *unindexed && *cols > 1 {
                    a[cols - 1].push_str(if st.take(4) == 0 { " unindexed" } else { " UNINDEXED" });
                }
                if *contentless {
                    a.push(st.option("content", "", false));
                    a.push(st.option("contentless_delete", "1", false));
                } else if st.take(5) == 0 {
                    a.push(st.option("contentless_delete", "0", false));
                }
                // A third of the tokenizers but trigram and Porter ones
                // wrapped in a Porter tokenizer (which stems the tokens of
                // the tokenizer it names, with its arguments).
                let t = tok.spelled(st.take(4));
                // A quarter of the other `unicode61` tokenizers with a
                // `categories` option too (`sqlite3Fts5UnicodeCatParse`):
                // letters, numbers and one to three of [`CATEGORIES`] (by
                // a spelling of its own, so that the others stay).
                let mut sc = Spelling(!spelling_seed(self, k));
                let t = if matches!(tok, Tok::Unicode61 | Tok::Diacritics | Tok::Porter | Tok::Tokenchars | Tok::PorterDiacritics) && sc.take(4) == 0 {
                    let more: String = (0..=sc.take(3)).map(|_| format!(" {}", CATEGORIES[usize::from(sc.take(CATEGORIES.len() as u64))])).collect();
                    let base = t.unwrap_or_else(|| "unicode61".into());
                    let base = if base == "porter" { "porter unicode61".into() } else { base };
                    Some(format!("{base} categories ''L* N*{more}''"))
                } else {
                    t
                };
                let t = if !tok.trigram() && !self.porter() && st.take(3) == 0 { Some(format!("porter {}", t.as_deref().unwrap_or("unicode61"))) } else { t };
                if let Some(t) = t {
                    a.push(st.option("tokenize", &t, true));
                }
                if *prefix != 0 {
                    let p = PREFIXES[usize::from(*prefix) % PREFIXES.len()];
                    for one in p.split(", ") {
                        let v = one.trim_start_matches("prefix=").trim_matches('\'');
                        let v = match st.take(4) {
                            0 if v.contains(' ') => v.replace(' ', ","),
                            1 if v.contains(' ') => format!(" {} ", v.replace(' ', " , ")),
                            _ => v.to_string(),
                        };
                        a.push(st.option("prefix", &v, false));
                    }
                }
                for (j, o) in OPTS.iter().enumerate() {
                    let name = o.trim_end_matches("=1");
                    if opts >> j & 1 != 0 {
                        a.push(st.option(name, "1", false));
                    } else if j < 2 && st.take(5) == 0 {
                        a.push(st.option(name, "0", false));
                    }
                }
                match detail {
                    Detail::Full if st.take(4) == 0 => a.push(st.option("detail", "full", false)),
                    Detail::Full => {},
                    Detail::Column => a.push(st.option("detail", "column", false)),
                    Detail::None => a.push(st.option("detail", "none", false)),
                }
                if !columnsize {
                    a.push(st.option("columnsize", "0", false));
                } else if st.take(4) == 0 {
                    a.push(st.option("columnsize", "1", false));
                }
                format!("fts5({})", a.join(", "))
            },
            Kind::Vocab { of, form } => format!("fts5vocab(vt{of}, '{}')", ["row", "col", "instance"][usize::from(*form % 3)]),
        };
        format!("CREATE VIRTUAL TABLE vt{k} USING {args}")
    }

    /// How many values a row holds (the model's; none for a vocabulary).
    pub fn width(&self) -> usize {
        match self {
            Kind::Rtree { dims, aux, .. } => 2 * dims + aux,
            Kind::Geopoly { aux } => 1 + aux,
            Kind::Fts5 { cols, .. } => *cols,
            Kind::Vocab { .. } => 0,
        }
    }

    pub fn fts(&self) -> bool {
        matches!(self, Kind::Fts5 { .. })
    }

    fn contentless(&self) -> bool {
        matches!(self, Kind::Fts5 { contentless: true, .. })
    }

    fn locale(&self) -> bool {
        matches!(self, Kind::Fts5 { opts, .. } if opts & 2 != 0)
    }

    /// An FTS5 table of a Porter tokenizer.
    fn porter(&self) -> bool {
        matches!(self, Kind::Fts5 { tok: Tok::Porter | Tok::PorterAscii | Tok::PorterDiacritics, .. })
    }

    /// Whether column `c` reads back NULL: a contentless table's, but an
    /// UNINDEXED one with `contentless_unindexed=1`.
    fn reads_null(&self, c: usize) -> bool {
        match self {
            Kind::Fts5 { contentless: true, cols, unindexed, opts, .. } => !(opts & 4 != 0 && *unindexed && c + 1 == *cols),
            _ => false,
        }
    }
}

/// A row: its rowid (an R-Tree's `k`) and values (an R-Tree's coordinates
/// then its auxiliary columns; a Geopoly table's shape SQL then its
/// auxiliary columns; an FTS5 table's columns).
#[derive(Clone, Debug, PartialEq)]
pub struct VRow {
    pub id: i64,
    pub values: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VTable {
    pub kind: Kind,
    /// By rowid.
    pub rows: Vec<VRow>,
    /// Whether a row was replaced, updated or deleted since the CREATE or
    /// the last `rebuild` (an FTS5 index then may hold a rowid twice).
    pub rewritten: bool,
    /// Whether FTS5's `secure-delete` is on.
    pub secure: bool,
    /// Whether a row was replaced, updated or deleted with `secure-delete`
    /// off since the CREATE or the last `rebuild`: the index may hold a
    /// delete marker then.
    pub marked: bool,
}

impl VTable {
    fn has(&self, id: i64) -> bool {
        self.rows.binary_search_by_key(&id, |r| r.id).is_ok()
    }

    fn put(&mut self, row: VRow) {
        match self.rows.binary_search_by_key(&row.id, |r| r.id) {
            Ok(i) => self.rows[i] = row,
            Err(i) => self.rows.insert(i, row),
        }
    }

    /// The rows as `SELECT rowid, * FROM vt{k} ORDER BY rowid` gives them,
    /// a Geopoly table's `_shape` left out (`SELECT rowid, p0, ..`).
    pub fn contents(&self) -> Vec<Vec<Value>> {
        self.rows
            .iter()
            .map(|r| {
                let skip = usize::from(matches!(self.kind, Kind::Geopoly { .. }));
                let mut v = vec![Value::Int(r.id)];
                if let Kind::Rtree { .. } = self.kind {
                    v.push(Value::Int(r.id));
                }
                if self.kind.contentless() {
                    v.extend(r.values.iter().enumerate().map(|(c, x)| if self.kind.reads_null(c) { Value::Null } else { x.clone() }));
                } else {
                    v.extend(r.values[skip..].iter().cloned());
                }
                v
            })
            .collect()
    }

    /// The query whose rows [`VTable::contents`] gives.
    pub fn contents_sql(&self, k: usize) -> String {
        match &self.kind {
            Kind::Geopoly { .. } => {
                let cols = self.kind.columns()[1..].to_vec();
                format!("SELECT rowid{} FROM vt{k} ORDER BY rowid", cols.iter().map(|c| format!(", {c}")).collect::<String>())
            },
            _ => format!("SELECT rowid, * FROM vt{k} ORDER BY rowid"),
        }
    }
}

/// An FTS5 maintenance command (`INSERT INTO vt{k}(vt{k}[, rank]) VALUES
/// (..)`).
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Optimize,
    Rebuild,
    Integrity,
    /// `('merge', n)`, `automerge`, `usermerge`, `crisismerge`, `pgsz`,
    /// `deletemerge`: CONFIGS`[k]`.
    Config(usize, i64),
    /// `('rank', 'bm25(..)')`.
    Rank(i64, i64),
    /// `delete-all`, of a contentless table: every row gone.
    DeleteAll,
}

/// Each configuration value of `sqlite3Fts5ConfigSetValue` with the
/// values drawn for it (`merge` a command): `automerge` 1 (the default,
/// 4), `crisismerge` 0 and 1 (the default, 16) and past the most segments
/// (clamped to 1999), `deletemerge` past 100 (0), `secure-delete` and
/// `insttoken` any number not 0 (on), `hashsize` the bytes of pending
/// terms before a flush.
const CONFIGS: [(&str, i64, i64); 9] = [
    ("merge", -16, 16),
    ("automerge", 0, 16),
    ("usermerge", 2, 16),
    ("crisismerge", 0, 2100),
    ("pgsz", 32, 4000),
    ("deletemerge", 0, 120),
    ("secure-delete", 0, 3),
    ("hashsize", 64, 100_000),
    ("insttoken", 0, 2),
];

/// A statement on a virtual table.
#[derive(Clone, Debug, PartialEq)]
pub enum VStmt {
    Create { vt: usize, kind: Kind },
    /// `DROP TABLE vt{k}`, of the last.
    Drop(usize),
    /// `INSERT [OR REPLACE] INTO vt{k}(rowid, ..) VALUES ..`.
    Insert { vt: usize, rows: Vec<VRow>, replace: bool },
    /// An R-Tree's, Geopoly or FTS5 table's rows `start + i` for `i` below `count`
    /// from a recursive CTE ([`fill_row`]), `mul` their spread.
    Fill { vt: usize, start: i64, count: i64, mul: i64, replace: bool },
    /// `DELETE FROM vt{k} WHERE rowid BETWEEN lo AND hi`.
    Delete { vt: usize, lo: i64, hi: i64 },
    /// `UPDATE vt{k} SET .. WHERE rowid = id` (an R-Tree's `k` as a set
    /// column 0 moves the row to id `to`).
    Update { vt: usize, id: i64, sets: Vec<(usize, Value)> },
    /// `UPDATE vt{k} SET _shape = geopoly_xform(_shape, 1, 0, 0, 1, dx,
    /// dy) WHERE rowid BETWEEN lo AND hi`.
    Xform { vt: usize, lo: i64, hi: i64, dx: i64, dy: i64 },
    Command { vt: usize, cmd: Cmd },
    /// `SELECT rtreecheck('vt{k}')` of an R-Tree: `ok`.
    Check(usize),
}

// ------------------------------------------------------------------ values

/// The words FTS5 texts are made of, and what MATCH queries look for.
/// The second half are words of the suffixes the Porter stemmer's steps
/// take apart (`-ational`, `-izer`, `-iveness`, `-icate`, `-ement`, ..).
pub const WORDS: [&str; 48] = [
    "apple", "apricot", "banana", "band", "bandit", "cherry", "chess", "date", "delta", "echo", "eagle", "fig", "figure", "grape", "kiwi", "lemon", "running", "runs", "café", "naïve", "t1", "u3", "v5", "zeta",
    "relational", "conditional", "hesitanci", "digitizer", "operator", "feudalism", "decisiveness", "hopefulness", "callousness", "sensitiviti", "triplicate", "formative", "formalize", "electrical", "goodness", "revival",
    "allowance", "inference", "adjustable", "replacement", "adoption", "activate", "generalization", "caresses",
];

/// Words a Porter tokenizer's table's fill adds ([`fill_text`]): of the
/// suffixes of every step of the stemmer that [`WORDS`] lacks (`-enci`,
/// `-abli`, `-alli`, `-entli`, `-eli`, `-ousli`, `-ization`, `-ation`,
/// `-aliti`, `-biliti`, `-logi`, `-iciti`, `-ful`, `-er`, `-ic`, `-ible`,
/// `-ant`, `-ment`, `-ent`, `-ou`, `-ism`, `-iti`, `-ous`, `-ive`,
/// `-ize`; plurals, `-eed`, `-ed`, `-ing` with the endings step 1b
/// restores or undoubles, `-y`, a final `-e` and `-ll`), with stems of
/// measures 0 to 2, so that the step's conditions take both ways.
pub const PORTER_WORDS: [&str; 48] = [
    "valenci", "conformabli", "radicalli", "differentli", "vileli", "analogousli", "vietnamization", "predication", "formaliti", "sensibiliti", "archeologi", "electriciti",
    "hopeful", "airliner", "gyroscopic", "defensible", "irritant", "adjustment", "dependent", "homologou", "communism", "angulariti", "homologous", "effective",
    "bowdlerize", "ponies", "caress", "cats", "agreed", "feed", "plastered", "motoring", "sing", "happy", "conflated", "troubled",
    "sized", "hopping", "falling", "filing", "probate", "rate", "controll", "roll", "tanned", "sky", "ration", "ment",
];

/// Words every other FTS5 table's fill adds ([`fill_text`]): letters
/// past ASCII in every case SQLite folds (Latin, Greek, a titlecase
/// digraph, `ß`, a ligature), diacritics precomposed and combining (which
/// `remove_diacritics` takes off or keeps), CJK, digits and numbers of
/// other scripts (`Nd`, `Nl`, `No`), and characters the tokenizers split
/// on or keep by their categories (a no-break space, a zero-width space,
/// an emoji, `·`).
pub const UNICODE_WORDS: [&str; 24] = [
    "ÅNGSTRÖM", "Ωμέγα", "ǅemal", "straße", "ﬁle", "école", "e\u{301}cole", "nai\u{308}ve", "東京都", "١٢٣", "Ⅻ", "½x",
    "a\u{a0}b", "a\u{200b}b", "😀smile", "l·l", "ÇA", "ÿes", "Ǆ", "İstanbul", "ﬀ", "mañana", "ĳs", "ΣΑΣ",
];

/// The words [`fill_text`] adds to a table's fill: [`PORTER_WORDS`] for a
/// Porter tokenizer's, else [`UNICODE_WORDS`].
fn fill_words(kind: &Kind) -> &'static [&'static str] {
    if kind.porter() {
        &PORTER_WORDS
    } else {
        &UNICODE_WORDS
    }
}

/// The fill's words for row `i` of column `c`, then one of `more`.
fn fill_text(i: i64, mul: i64, c: usize, more: &[&str]) -> String {
    let w = |n: i64| WORDS[(n.rem_euclid(WORDS.len() as i64)) as usize];
    let text = match c {
        0 => format!("{} t{} {}", w(i * mul), i.rem_euclid(7), w(i + 3)),
        1 => format!("u{} {} v{}", (i * mul).rem_euclid(11), w(i * 5 + c as i64), i.rem_euclid(13)),
        _ => format!("{} {}", w(i * 7), w(i * mul + 1)),
    };
    format!("{text} {}", more[(i * mul + 17 * c as i64).rem_euclid(more.len() as i64) as usize])
}

/// The SQL of the fill's text of column `c` (what [`fill_text`] computes;
/// `WORDS` and `more` as JSON arrays read by `->>`).
fn fill_text_sql(mul: i64, c: usize, more: &[&str]) -> String {
    let n = WORDS.len();
    let w = |e: String| format!("(json('{}') ->> ((({e}) % {n} + {n}) % {n}))", words_json(&WORDS));
    let text = match c {
        0 => format!("{} || ' t' || (i % 7) || ' ' || {}", w(format!("i * {mul}")), w("i + 3".into())),
        1 => format!("'u' || (i * {mul} % 11) || ' ' || {} || ' v' || (i % 13)", w(format!("i * 5 + {c}"))),
        _ => format!("{} || ' ' || {}", w("i * 7".into()), w(format!("i * {mul} + 1"))),
    };
    let p = more.len();
    format!("{text} || ' ' || (json('{}') ->> (((i * {mul} + {}) % {p} + {p}) % {p}))", words_json(more), 17 * c)
}

fn words_json(words: &[&str]) -> String {
    format!("[{}]", words.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(","))
}

/// Coordinate `j` of fill row `i` of an R-Tree: a minimum for even `j`,
/// its maximum after it; halves for a float tree.
fn fill_coord(i: i64, mul: i64, j: usize, int: bool) -> Value {
    let d = (j / 2) as i64;
    let lo = (i * (mul + 2 * d)).rem_euclid(97 + 10 * d) - 20;
    let n = if j.is_multiple_of(2) { lo } else { lo + (i * 3 + d).rem_euclid(9) };
    if int {
        Value::Int(n)
    } else {
        Value::Real(n as f64 / 2.0)
    }
}

fn fill_coord_sql(mul: i64, j: usize, int: bool) -> String {
    let d = j / 2;
    let lo = format!("(i * {} % {} - 20)", mul + 2 * d as i64, 97 + 10 * d);
    let n = if j.is_multiple_of(2) { lo } else { format!("({lo} + (i * 3 + {d}) % 9)") };
    if int {
        n
    } else {
        format!("{n} / 2.0")
    }
}

/// Row `start + i` of a fill.
pub fn fill_row(kind: &Kind, start: i64, i: i64, mul: i64) -> VRow {
    let values = match kind {
        Kind::Rtree { dims, int, aux } => (0..2 * dims).map(|j| fill_coord(i, mul, j, *int)).chain((0..*aux).map(|a| if (i + a as i64) % 3 == 0 { Value::Null } else { Value::Int(i % 5 + a as i64) })).collect(),
        Kind::Fts5 { cols, .. } => (0..*cols).map(|c| Value::Text(fill_text(i, mul, c, fill_words(kind)))).collect(),
        Kind::Geopoly { aux } => std::iter::once(Value::Text(fill_shape(i, mul))).chain((0..*aux).map(|a| if (i + a as i64) % 3 == 0 { Value::Null } else { Value::Int(i % 5 + a as i64) })).collect(),
        _ => Vec::new(),
    };
    VRow { id: start + i, values }
}

/// The shape of fill row `i` of a Geopoly table: a regular polygon of 3 to
/// 8 sides about a point of a 41 by 37 grid (the model's shape SQL).
fn fill_shape(i: i64, mul: i64) -> String {
    format!("geopoly_regular({}, {}, {}, {})", (i * mul).rem_euclid(41) - 10, (i * 3 + mul).rem_euclid(37) - 10, 1 + i.rem_euclid(5), 3 + i.rem_euclid(6))
}

fn fill_shape_sql(mul: i64) -> String {
    format!("geopoly_regular((i * {mul} % 41) - 10, ((i * 3 + {mul}) % 37) - 10, 1 + i % 5, 3 + i % 6)")
}

fn fill_sql(kind: &Kind, vt: usize, start: i64, count: i64, mul: i64, replace: bool) -> String {
    let items: Vec<String> = match kind {
        Kind::Rtree { dims, int, aux } => {
            std::iter::once(format!("{start} + i")).chain((0..2 * dims).map(|j| fill_coord_sql(mul, j, *int))).chain((0..*aux).map(|a| format!("CASE WHEN (i + {a}) % 3 = 0 THEN NULL ELSE i % 5 + {a} END"))).collect()
        },
        // A `locale=1` table's first column as `fts5_locale(..)` for an
        // odd `mul`.
        Kind::Fts5 { cols, .. } => std::iter::once(format!("{start} + i")).chain((0..*cols).map(|c| if c == 0 && kind.locale() && mul % 2 == 1 { format!("fts5_locale('en', {})", fill_text_sql(mul, c, fill_words(kind))) } else { fill_text_sql(mul, c, fill_words(kind)) })).collect(),
        Kind::Geopoly { aux } => [format!("{start} + i"), fill_shape_sql(mul)].into_iter().chain((0..*aux).map(|a| format!("CASE WHEN (i + {a}) % 3 = 0 THEN NULL ELSE i % 5 + {a} END"))).collect(),
        _ => Vec::new(),
    };
    let cols = match kind {
        Kind::Fts5 { .. } | Kind::Geopoly { .. } => format!("(rowid, {})", kind.columns().join(", ")),
        _ => String::new(),
    };
    format!(
        "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i < {}) INSERT {}INTO vt{vt}{cols} SELECT {} FROM n",
        count - 1,
        if replace { "OR REPLACE " } else { "" },
        items.join(", ")
    )
}

/// An exactly representable coordinate.
fn coord(g: &mut Source, int: bool) -> Value {
    let n = g.int(-40i64..=60);
    if int {
        Value::Int(n)
    } else if g.bool_with(0.3) {
        Value::Real(n as f64 / 2.0)
    } else {
        Value::Real(n as f64)
    }
}

/// An auxiliary column's value.
fn aux_value(g: &mut Source) -> Value {
    match g.int(0..6) {
        0 => Value::Null,
        1 | 2 => Value::Int(g.int(-3i64..=9)),
        3 => Value::Real(g.int(-8i64..=8) as f64 / 4.0),
        _ => Value::Text(g.pick(&WORDS).to_string()),
    }
}

/// An FTS5 column's text: 0 to 8 words, now and then capitalized or with
/// punctuation; else NULL or an integer.
fn fts_value(g: &mut Source) -> Value {
    match g.int(0..20) {
        0 => Value::Null,
        1 => Value::Int(g.int(0i64..=2024)),
        _ => {
            let n = g.int(0..=8);
            let mut s = String::new();
            for k in 0..n {
                if k > 0 {
                    s.push_str(g.pick(&[" ", " ", " ", ", ", "-", ". "]));
                }
                let w = g.pick(&WORDS);
                if g.bool_with(0.15) {
                    s.push_str(&w.to_uppercase());
                } else {
                    s.push_str(w);
                }
            }
            Value::Text(s)
        },
    }
}

/// A valid polygon's SQL: a regular polygon or a JSON ring (a triangle or
/// rectangle, counter-clockwise or not).
pub fn shape_sql(g: &mut Source) -> String {
    let x = g.int(-10i64..=20);
    let y = g.int(-10i64..=20);
    match g.int(0..3) {
        0 => format!("geopoly_regular({x}, {y}, {}, {})", g.int(1i64..=6), g.int(3i64..=9)),
        1 => {
            let (w, h) = (g.int(1i64..=8), g.int(1i64..=8));
            let mut pts = vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
            if g.bool() {
                pts.reverse();
            }
            pts.push(pts[0]);
            format!("'[{}]'", pts.iter().map(|(a, b)| format!("[{a},{b}]")).collect::<Vec<_>>().join(","))
        },
        _ => {
            let (w, h) = (g.int(1i64..=8), g.int(1i64..=8));
            format!("'[[{x},{y}],[{},{y}],[{},{}],[{x},{y}]]'", x + w, x + w / 2, y + h)
        },
    }
}

fn row_values(g: &mut Source, kind: &Kind) -> Vec<Value> {
    match kind {
        Kind::Rtree { dims, int, aux } => {
            let mut v = Vec::new();
            for _ in 0..*dims {
                let a = coord(g, *int);
                let b = coord(g, *int);
                let (lo, hi) = if num(&a) <= num(&b) { (a, b) } else { (b, a) };
                v.push(lo);
                v.push(hi);
            }
            v.extend((0..*aux).map(|_| aux_value(g)));
            v
        },
        Kind::Geopoly { aux } => std::iter::once(Value::Text(shape_sql(g))).chain((0..*aux).map(|_| aux_value(g))).collect(),
        Kind::Fts5 { cols, .. } => (0..*cols).map(|_| fts_value(g)).collect(),
        Kind::Vocab { .. } => Vec::new(),
    }
}

/// An integer of one of the ranges, picked by weight.
fn ranged(g: &mut Source, ranges: &[(u32, i64, i64)]) -> i64 {
    let k = *g.weighted(&ranges.iter().enumerate().map(|(k, r)| (r.0, k)).collect::<Vec<_>>());
    g.int(ranges[k].1..=ranges[k].2)
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Int(i) => *i as f64,
        Value::Real(x) => *x,
        _ => 0.0,
    }
}

fn fresh_id(g: &mut Source, t: &VTable) -> i64 {
    for _ in 0..8 {
        let id = if g.bool_with(0.8) { t.rows.last().map_or(1, |r| r.id + 1) + g.int(0i64..=2) } else { g.int(-5i64..=500) };
        if !t.has(id) {
            return id;
        }
    }
    t.rows.last().map_or(1, |r| r.id + 1)
}

pub fn draw_kind(g: &mut Source, fts: &[usize]) -> Kind {
    match g.int(0..20) {
        0..=5 => Kind::Rtree { dims: *g.weighted(&[(3, 1usize), (5, 2), (2, 3), (1, 4), (1, 5)]), int: g.bool_with(0.25), aux: *g.weighted(&[(3, 0usize), (2, 1), (1, 2)]) },
        6..=8 => Kind::Geopoly { aux: g.int(0..=2) },
        9..=12 if !fts.is_empty() => Kind::Vocab { of: *g.pick(fts), form: g.int(0..3) },
        _ => {
            let tok = *g.weighted(&[(5, Tok::Unicode61), (2, Tok::Diacritics), (2, Tok::Ascii), (3, Tok::Porter), (1, Tok::PorterAscii), (2, Tok::Trigram), (1, Tok::TrigramCase), (1, Tok::Tokenchars), (1, Tok::Categories(0)), (1, Tok::AsciiSeparators), (1, Tok::PorterDiacritics), (1, Tok::TrigramDiacritics)]);
            let tok = if let Tok::Categories(_) = tok { Tok::Categories((0..g.int(0..=3)).fold(0, |m, _| m | 1 << g.index(CATEGORIES.len()))) } else { tok };
            let detail = if tok.trigram() { Detail::Full } else { *g.weighted(&[(5, Detail::Full), (2, Detail::Column), (2, Detail::None)]) };
            let columnsize = g.bool_with(0.8);
            let (cols, contentless, unindexed) = (*g.weighted(&[(2, 1usize), (5, 2), (2, 3)]), columnsize && g.bool_with(0.25), g.bool_with(0.2));
            let prefix = if g.bool_with(0.35) { g.int(1..PREFIXES.len() as u8) } else { 0 };
            let mut opts = u8::from(g.bool_with(0.2)) | u8::from(g.bool_with(0.35)) << 1;
            if contentless && unindexed && cols > 1 && opts & 2 == 0 && g.bool_with(0.5) {
                opts |= 4;
            }
            Kind::Fts5 { cols, tok, prefix, detail, columnsize, contentless, unindexed, opts }
        },
    }
}

/// A LIKE or GLOB of word `w` (`k` picks the form), which a trigram
/// table answers from its index (`sqlite3Fts5ExprPattern`: the runs of 3
/// characters or more between wildcards): `LIKE '%abc%'`, a prefix
/// (`LIKE 'abc%'`), `_` inside a run (`LIKE '%a_cd%'`), a longer run
/// (`LIKE '%abcdef%'`, of a short word all of it), `GLOB '*abc*'`,
/// `GLOB 'ab?d*'`, a set (`GLOB '*[a-c]pple*'`).
fn trigram_pattern(w: &str, k: u8) -> String {
    let c: Vec<char> = w.chars().collect();
    let take = |n: usize| c.iter().take(n).collect::<String>();
    let mask = |at: usize, with: char, n: usize| c.iter().take(n).enumerate().map(|(i, &x)| if i == at { with } else { x }).collect::<String>();
    match k % 8 {
        1 => format!("LIKE '{}%'", take(3)),
        2 if c.len() >= 4 => format!("LIKE '%{}%'", mask(1, '_', 4)),
        3 => format!("LIKE '%{}%'", take(6)),
        4 => format!("GLOB '*{}*'", take(3)),
        5 if c.len() >= 4 => format!("GLOB '{}*'", mask(2, '?', 4)),
        6 if c.len() >= 4 => format!("GLOB '*[a-c]{}*'", c[1..].iter().take(4).collect::<String>()),
        _ => format!("LIKE '%{}%'", take(3)),
    }
}

/// Whether `s` may run in a database of encoding `utf16` (UTF-16 or
/// not): no fts5vocab table in a UTF-16 one. The table takes a range of
/// `term` (its own constraint's, or a query's around it that the planner
/// pushes down to it) in the terms' UTF-8 order, while SQLite compares
/// texts in the database's encoding, and the two orders differ past
/// U+00FF (`ĳs`, `東京都`, an emoji, of [`UNICODE_WORDS`]): `term < 'b'`
/// gives `apple` alone, `+term < 'b'` `ĳs` too (trunk 185ffd27064b and
/// 3.51.0).
pub fn vocab_ok(s: &VStmt, utf16: bool) -> bool {
    !(utf16 && matches!(s, VStmt::Create { kind: Kind::Vocab { .. }, .. }))
}

/// The statements that make a virtual table and fill it: dozens to
/// hundreds of rows, for interior R-Tree nodes and FTS5 segments.
pub fn draw_made(g: &mut Source, vtabs: &[VTable]) -> Vec<VStmt> {
    let fts: Vec<usize> = (0..vtabs.len()).filter(|&k| vtabs[k].kind.fts()).collect();
    let kind = draw_kind(g, &fts);
    let vt = vtabs.len();
    let mut out = vec![VStmt::Create { vt, kind: kind.clone() }];
    let mut t = VTable { kind: kind.clone(), rows: Vec::new(), rewritten: false, secure: false, marked: false };
    if let Kind::Fts5 { .. } = kind {
        // Small pages 60% of the time (40% before): more leaves, segments
        // and merges for the same rows.
        if g.bool_with(0.6) {
            out.push(VStmt::Command { vt, cmd: Cmd::Config(4, *g.pick(&[32, 64, 128, 512])) });
        }
        if g.bool_with(0.2) {
            out.push(VStmt::Command { vt, cmd: Cmd::Config(1, g.int(0..=4)) });
        }
        // `secure-delete` half the time (a quarter before; the draw is
        // the same): deletes then rewrite the leaves in place
        // (`fts5DoSecureDelete`), else leave tombstones.
        if g.bool_with(0.5) {
            out.push(VStmt::Command { vt, cmd: Cmd::Config(6, 1) });
        }
    }
    if matches!(kind, Kind::Vocab { .. }) {
        return out;
    }
    for _ in 0..g.int(1..=3) {
        let s = if g.bool_with(0.5) {
            let count = ranged(g, &[(3, 5, 40), (3, 40, 200), (1, 200, 350)]);
            let start = t.rows.last().map_or(g.int(-3i64..=10), |r| r.id + 1);
            VStmt::Fill { vt, start, count: count.min((MAX_ROWS - t.rows.len()) as i64), mul: g.int(1i64..=13), replace: false }
        } else {
            draw_insert(g, &t, vt)
        };
        if let VStmt::Fill { count, .. } = &s {
            if *count < 1 {
                continue;
            }
        }
        apply_one(&mut t, &s);
        out.push(s);
    }
    // Now and then a tenth to two thirds of the rows deleted, a range of
    // rowids: an R-Tree's leaves and nodes underflow (CondenseTree removes
    // them and reinserts their cells), an FTS5 index takes tombstones and
    // deletes (secure-delete's in place); `rtreecheck` or integrity-check
    // after it 60% of the time (30% before; the same draw).
    if t.rows.len() >= 20 && g.bool_with(0.4) {
        let n = t.rows.len();
        let i = g.index(n);
        let span = n * g.int(10usize..=66) / 100;
        let s = VStmt::Delete { vt, lo: t.rows[i].id, hi: t.rows[(i + span).min(n - 1)].id };
        apply_one(&mut t, &s);
        out.push(s);
        if g.bool_with(0.6) {
            out.push(match kind {
                Kind::Fts5 { .. } => VStmt::Command { vt, cmd: Cmd::Integrity },
                Kind::Rtree { .. } => VStmt::Check(vt),
                _ => return out,
            });
        }
    }
    out
}

fn draw_insert(g: &mut Source, t: &VTable, vt: usize) -> VStmt {
    let n = ranged(g, &[(3, 1, 1), (3, 2, 10), (2, 10, 40)]) as usize;
    let n = n.min(MAX_ROWS.saturating_sub(t.rows.len())).max(1);
    let replace = !matches!(t.kind, Kind::Geopoly { .. }) && !t.rows.is_empty() && g.bool_with(0.15);
    let mut scratch = t.clone();
    let mut rows = Vec::new();
    for _ in 0..n {
        let id = if replace && g.bool_with(0.5) { g.pick(&scratch.rows).id } else { fresh_id(g, &scratch) };
        let row = VRow { id, values: row_values(g, &t.kind) };
        scratch.put(row.clone());
        rows.push(row);
    }
    VStmt::Insert { vt, rows, replace }
}

/// A statement on one of `vtabs` (or a new one), valid there.
pub fn draw(g: &mut Source, vtabs: &[VTable], open: bool) -> Option<VStmt> {
    if vtabs.is_empty() || (vtabs.len() < MAX_VTABS && g.bool_with(0.08)) {
        return (vtabs.len() < MAX_VTABS).then(|| draw_made(g, vtabs).remove(0));
    }
    let vt = g.index(vtabs.len());
    let t = &vtabs[vt];
    let ids = |g: &mut Source| -> (i64, i64) {
        if t.rows.is_empty() {
            return (0, 3);
        }
        let a = g.pick(&t.rows).id;
        (a, a + ranged(g, &[(3, 0, 0), (2, 1, 5), (1, 5, 100), (1, 20, 300)]))
    };
    let s = match (&t.kind, g.int(0..12)) {
        (_, _) if !open && vt + 1 == vtabs.len() && g.bool_with(0.04) => VStmt::Drop(vt),
        (Kind::Vocab { .. }, _) if !open && g.bool_with(0.3) && vt + 1 == vtabs.len() => VStmt::Drop(vt),
        (Kind::Vocab { .. }, _) => return None,
        (_, 0..=3) => draw_insert(g, t, vt),
        (Kind::Rtree { .. } | Kind::Fts5 { .. } | Kind::Geopoly { .. }, 4) if t.rows.len() + 5 <= MAX_ROWS => {
            let count = g.int(5i64..=(MAX_ROWS - t.rows.len()).min(120) as i64);
            let replace = !matches!(t.kind, Kind::Geopoly { .. }) && g.bool_with(0.2);
            let start = if replace && !t.rows.is_empty() { g.pick(&t.rows).id } else { t.rows.last().map_or(1, |r| r.id + 1) };
            VStmt::Fill { vt, start, count, mul: g.int(1i64..=13), replace }
        },
        (_, 4..=6) => {
            let (lo, hi) = ids(g);
            VStmt::Delete { vt, lo, hi }
        },
        (Kind::Geopoly { .. }, 7) => {
            let (lo, hi) = ids(g);
            VStmt::Xform { vt, lo, hi, dx: g.int(-3i64..=3), dy: g.int(-3i64..=3) }
        },
        (Kind::Rtree { .. }, 7) => VStmt::Check(vt),
        (Kind::Fts5 { .. }, 7..=8) => VStmt::Command {
            vt,
            cmd: match g.int(0..10) {
                0 => Cmd::Optimize,
                1 if t.kind.contentless() => Cmd::DeleteAll,
                1 => Cmd::Rebuild,
                2 => Cmd::Integrity,
                3 => Cmd::Rank(g.int(1i64..=10), g.int(1i64..=10)),
                _ => {
                    let k = g.index(CONFIGS.len());
                    Cmd::Config(k, g.int(CONFIGS[k].1..=CONFIGS[k].2))
                },
            },
        },
        _ if !t.rows.is_empty() => {
            let r = g.pick(&t.rows).clone();
            let fresh = row_values(g, &t.kind);
            let mut sets = Vec::new();
            match &t.kind {
                Kind::Rtree { dims, .. } => {
                    // A dimension's pair, or one bound kept within the other.
                    let d = g.index(*dims);
                    if g.bool() {
                        sets.push((2 * d, fresh[2 * d].clone()));
                        sets.push((2 * d + 1, fresh[2 * d + 1].clone()));
                    } else if num(&fresh[2 * d]) <= num(&r.values[2 * d + 1]) {
                        sets.push((2 * d, fresh[2 * d].clone()));
                    }
                    for (j, v) in fresh.iter().enumerate().skip(2 * dims) {
                        if g.bool() {
                            sets.push((j, v.clone()));
                        }
                    }
                },
                _ => {
                    // A Geopoly table's auxiliary columns alone half the
                    // time (its `xUpdate` keeps the shape unchanged).
                    let all = t.kind.contentless();
                    let geo = matches!(t.kind, Kind::Geopoly { .. });
                    let shape = matches!(t.kind, Kind::Geopoly { aux } if aux == 0 || g.bool());
                    for (j, v) in fresh.into_iter().enumerate() {
                        if if geo && j == 0 { shape } else { all || g.bool() } {
                            sets.push((j, v));
                        }
                    }
                },
            }
            if sets.is_empty() {
                return None;
            }
            VStmt::Update { vt, id: r.id, sets }
        },
        _ => return None,
    };
    valid(vtabs, &s, open).then_some(s)
}

// ------------------------------------------------------------- semantics

/// Whether a ROLLBACK TO a savepoint whose model kept `saved` may run:
/// not once an FTS5 table was made after it (the rollback drops it, and the
/// next SAVEPOINT fails, "SQL logic error", 3.51.0 and trunk 5bd093e261a7;
/// bugs/sqlite/sqlite-vtab-create-rollback-to-savepoint).
pub fn rollback_to_ok(saved: &[VTable], now: &[VTable]) -> bool {
    now.iter().skip(saved.len()).all(|t| !t.kind.fts())
}

/// The kind of the table it writes (its own for a CREATE; none for a
/// table not there).
pub fn target(vtabs: &[VTable], s: &VStmt) -> Option<Kind> {
    match s {
        VStmt::Create { kind, .. } => Some(kind.clone()),
        VStmt::Drop(vt) | VStmt::Check(vt) | VStmt::Insert { vt, .. } | VStmt::Fill { vt, .. } | VStmt::Delete { vt, .. } | VStmt::Update { vt, .. } | VStmt::Xform { vt, .. } | VStmt::Command { vt, .. } => vtabs.get(*vt).map(|t| t.kind.clone()),
    }
}

/// Whether `kind` is the kind of the table it writes.
pub fn kind_ok(vtabs: &[VTable], s: &VStmt, kind: &Kind) -> bool {
    target(vtabs, s).as_ref() == Some(kind)
}

/// Whether SQLite runs it against `vtabs` (`open`: inside a transaction).
pub fn valid(vtabs: &[VTable], s: &VStmt, open: bool) -> bool {
    let t = |k: usize| vtabs.get(k);
    match s {
        VStmt::Create { vt, kind } => {
            *vt == vtabs.len()
                && *vt < MAX_VTABS
                && match kind {
                    Kind::Rtree { dims, aux, .. } => (1..=5).contains(dims) && *aux <= 3,
                    Kind::Geopoly { aux } => *aux <= 3,
                    // "contentless_delete=1 is incompatible with columnsize=0".
                    // contentless_unindexed=1 only of a contentless table
                    // ("contentless_unindexed=1 requires a contentless
                    // table") with an UNINDEXED column, and not with
                    // locale=1: an INSERT fails ("database disk image is
                    // malformed", 3.51.0 and trunk 185ffd27064b;
                    // bugs/sqlite/sqlite-fts5-locale-contentless-unindexed-insert).
                    Kind::Fts5 { cols, tok, detail, contentless, columnsize, prefix, unindexed, opts } => {
                        (1..=3).contains(cols) && (!tok.trigram() || *detail == Detail::Full) && (!contentless || *columnsize) && usize::from(*prefix) < PREFIXES.len() && *opts < 8 && (opts & 4 == 0 || *contentless && *unindexed && *cols > 1 && opts & 2 == 0)
                    },
                    Kind::Vocab { of, form } => *form < 3 && t(*of).is_some_and(|f| f.kind.fts()),
                }
        },
        // Never inside a transaction: a vocabulary table over it would be
        // left reading a table rolled back to nothing. Nor of a
        // contentless_unindexed=1 table with an UNINDEXED column: its
        // `%_content` stays, and the next table of its name fails ("table
        // 'vt0_content' already exists", trunk 185ffd27064b; the known
        // bugs/sqlite/sqlite-fts5-contentless-unindexed-drop-rename).
        VStmt::Drop(k) => *k + 1 == vtabs.len() && !open && !matches!(vtabs[*k].kind, Kind::Fts5 { opts, unindexed: true, cols, .. } if opts & 4 != 0 && cols > 1),
        VStmt::Insert { vt, rows, replace } => t(*vt).is_some_and(|t| {
            let mut s = t.clone();
            !matches!(t.kind, Kind::Vocab { .. })
                && !rows.is_empty()
                && !(*replace && matches!(t.kind, Kind::Geopoly { .. }))
                && rows.iter().all(|r| !matches!(t.kind, Kind::Geopoly { .. }) || matches!(r.values.first(), Some(Value::Text(_))))
                && rows.iter().all(|r| {
                    let ok = row_ok(&t.kind, r) && (*replace || !s.has(r.id));
                    s.put(r.clone());
                    ok
                })
                && s.rows.len() <= MAX_ROWS
        }),
        VStmt::Fill { vt, start, count, replace, .. } => t(*vt).is_some_and(|t| {
            (matches!(t.kind, Kind::Rtree { .. } | Kind::Fts5 { .. }) || matches!(t.kind, Kind::Geopoly { .. }) && !*replace)
                && *count >= 1
                && start.abs() < 1 << 30
                && (*replace || (0..*count).all(|i| !t.has(start + i)))
                && t.rows.len() + *count as usize <= MAX_ROWS
        }),
        VStmt::Delete { vt, lo, hi } => t(*vt).is_some_and(|t| !matches!(t.kind, Kind::Vocab { .. }) && lo <= hi),
        VStmt::Update { vt, id, sets } => t(*vt).is_some_and(|t| {
            let Some(r) = t.rows.iter().find(|r| r.id == *id) else { return false };
            let mut r = r.clone();
            !sets.is_empty()
                && sets.iter().all(|(c, v)| {
                    let ok = *c < r.values.len();
                    if ok {
                        r.values[*c] = v.clone();
                    }
                    ok
                })
                && row_ok(&t.kind, &r)
                // A contentless table's UPDATE sets every column.
                && (!t.kind.contentless() || (0..r.values.len()).all(|c| sets.iter().any(|(d, _)| *d == c)))
                && !matches!(t.kind, Kind::Geopoly { .. } if sets.iter().any(|(c, v)| *c == 0 && !matches!(v, Value::Text(_))))
        }),
        VStmt::Xform { vt, lo, hi, .. } => t(*vt).is_some_and(|t| matches!(t.kind, Kind::Geopoly { .. }) && lo <= hi),
        VStmt::Command { vt, cmd } => t(*vt).is_some_and(|t| {
            t.kind.fts()
                && match cmd {
                    // No `pgsz` for a tokendata=1 table: with small pages a
                    // MATCH of a term the index lacks fails ("fts5:
                    // corruption found reading blob", 3.51.0 and trunk
                    // 185ffd27064b; bugs/sqlite/
                    // sqlite-fts5-tokendata-missing-term-dlidx-corrupt).
                    // No `secure-delete` turned on in a detail=none table
                    // whose rows were replaced, updated or deleted (since
                    // the last `rebuild`): a rowid replaced before and
                    // deleted after corrupts the index ("fts5: checksum
                    // mismatch", MATCH finds rowids that never were; 3.51.0
                    // and trunk 185ffd27064b).
                    Cmd::Config(k, n) => *k < CONFIGS.len() && (CONFIGS[*k].1..=CONFIGS[*k].2).contains(n) && !(*k == 4 && matches!(t.kind, Kind::Fts5 { opts, .. } if opts & 1 != 0))
                        && !(*k == 6 && *n != 0 && t.rewritten && matches!(t.kind, Kind::Fts5 { detail: Detail::None, .. }))
                        // Nor turned on once a delete marker may have been
                        // written with it off: a merge can drop the marker
                        // and a later secure delete then shows the older
                        // entry again (MATCH finds the rowid, "fts5:
                        // checksum mismatch"; any detail; 3.51.0 and trunk
                        // 185ffd27064b; the known SQLite bug
                        // sqlite-fts5-secure-delete-merge-drops-marker).
                        && !(*k == 6 && *n != 0 && t.marked),
                    Cmd::Rank(a, b) => (1..=10).contains(a) && (1..=10).contains(b),
                    // "'rebuild' may not be used with a contentless fts5 table".
                    Cmd::Rebuild => !t.kind.contentless(),
                    Cmd::DeleteAll => t.kind.contentless(),
                    _ => true,
                }
        }),
        VStmt::Check(k) => t(*k).is_some_and(|t| matches!(t.kind, Kind::Rtree { .. })),
    }
}

fn row_ok(kind: &Kind, r: &VRow) -> bool {
    let exact = |v: &Value, int: bool| match v {
        Value::Int(i) => i.abs() < 1 << 20,
        Value::Real(x) => !int && x.abs() < (1u64 << 20) as f64 && (*x * 2.0).fract() == 0.0,
        _ => false,
    };
    r.values.len() == kind.width()
        && match kind {
            Kind::Rtree { dims, int, .. } => (0..*dims).all(|d| exact(&r.values[2 * d], *int) && exact(&r.values[2 * d + 1], *int) && num(&r.values[2 * d]) <= num(&r.values[2 * d + 1])),
            // NULL: moved by `geopoly_xform`, still valid. A text only of
            // a shape's SQL (not an FTS5 table's text a mutated case put
            // there).
            Kind::Geopoly { .. } => match &r.values[0] {
                Value::Text(t) => t.starts_with("geopoly_regular(") || t.starts_with("'[["),
                v => *v == Value::Null,
            },
            Kind::Fts5 { .. } => r.values.iter().all(|v| matches!(v, Value::Null | Value::Int(_) | Value::Text(_))),
            Kind::Vocab { .. } => false,
        }
        && r.values.iter().all(|v| !matches!(v, Value::Blob(_)))
}

/// The stored value of an R-Tree coordinate: REAL for `rtree`.
fn stored(kind: &Kind, c: usize, v: &Value) -> Value {
    match (kind, v) {
        (Kind::Rtree { dims, int: false, .. }, Value::Int(i)) if c < 2 * dims => Value::Real(*i as f64),
        _ => v.clone(),
    }
}

fn apply_one(t: &mut VTable, s: &VStmt) {
    match s {
        VStmt::Insert { rows, .. } => {
            for r in rows {
                let replaced = t.has(r.id);
                t.rewritten |= replaced;
                t.marked |= replaced && !t.secure;
                let values = r.values.iter().enumerate().map(|(c, v)| stored(&t.kind, c, v)).collect();
                t.put(VRow { id: r.id, values });
            }
        },
        VStmt::Fill { start, count, mul, .. } => {
            for i in 0..*count {
                let r = fill_row(&t.kind, *start, i, *mul);
                let replaced = t.has(r.id);
                t.rewritten |= replaced;
                t.marked |= replaced && !t.secure;
                t.put(r);
            }
        },
        VStmt::Delete { lo, hi, .. } => {
            t.rewritten = true;
            t.marked |= !t.secure;
            t.rows.retain(|r| r.id < *lo || r.id > *hi);
        },
        VStmt::Update { id, sets, .. } => {
            t.rewritten = true;
            t.marked |= !t.secure;
            let kind = t.kind.clone();
            if let Some(r) = t.rows.iter_mut().find(|r| r.id == *id) {
                for (c, v) in sets {
                    r.values[*c] = stored(&kind, *c, v);
                }
            }
        },
        VStmt::Xform { lo, hi, .. } => {
            for r in &mut t.rows {
                if r.id >= *lo && r.id <= *hi {
                    r.values[0] = Value::Null;
                }
            }
        },
        _ => {},
    }
}

/// Its effect on `vtabs`.
pub fn apply(vtabs: &mut Vec<VTable>, s: &VStmt) {
    match s {
        VStmt::Create { kind, .. } => vtabs.push(VTable { kind: kind.clone(), rows: Vec::new(), rewritten: false, secure: false, marked: false }),
        VStmt::Drop(_) => {
            vtabs.pop();
        },
        VStmt::Insert { vt, .. } | VStmt::Fill { vt, .. } | VStmt::Delete { vt, .. } | VStmt::Update { vt, .. } | VStmt::Xform { vt, .. } => apply_one(&mut vtabs[*vt], s),
        VStmt::Command { vt, cmd: Cmd::DeleteAll } => vtabs[*vt].rows.clear(),
        VStmt::Command { vt, cmd: Cmd::Rebuild } => {
            vtabs[*vt].rewritten = false;
            vtabs[*vt].marked = false;
        },
        VStmt::Command { vt, cmd: Cmd::Config(6, n) } => vtabs[*vt].secure = *n != 0,
        VStmt::Command { .. } | VStmt::Check(_) => {},
    }
}

/// The rows a statement answers, when the model knows them: `ok` for
/// `rtreecheck`.
pub fn answer(s: &VStmt) -> Option<Vec<Vec<Value>>> {
    matches!(s, VStmt::Check(_)).then(|| vec![vec![Value::Text("ok".into())]])
}

/// A value as an INSERT or UPDATE of a `kind` table writes it: a text of a
/// `locale=1` table now and then as `fts5_locale(..)` (by its length,
/// which shrinking keeps), which reads back as the text.
fn lit(kind: &Kind, v: &Value) -> String {
    match v {
        Value::Text(t) if kind.locale() && t.len() % 3 != 1 => format!("fts5_locale('{}', {})", ["en", "de_DE", ""][t.len() % 3], literal(v)),
        v => literal(v),
    }
}

fn sql(s: &VStmt, kind: &Kind) -> String {
    match s {
        VStmt::Create { vt, kind } => kind.create_sql(*vt),
        VStmt::Drop(k) => format!("DROP TABLE vt{k}"),
        VStmt::Insert { vt, rows, replace } => {
            let or = if *replace { "OR REPLACE " } else { "" };
            let values: Vec<String> = rows
                .iter()
                .map(|r| {
                    // A Geopoly shape is its SQL (only a Geopoly table's: a
                    // mutated case may insert a Geopoly table's rows into
                    // an FTS5 table of its name, which stores the text).
                    let geo = matches!(kind, Kind::Geopoly { .. });
                    let vals = r.values.iter().enumerate().map(|(c, v)| match v {
                        Value::Text(t) if geo && c == 0 => t.clone(),
                        v => lit(kind, v),
                    });
                    format!("({}, {})", r.id, vals.collect::<Vec<_>>().join(", "))
                })
                .collect();
            // The column list is the kind's: rendered by the caller.
            format!("INSERT {or}INTO vt{vt} VALUES {}", values.join(", "))
        },
        VStmt::Fill { .. } => unreachable!("rendered with its kind"),
        VStmt::Delete { vt, lo, hi } if lo == hi => format!("DELETE FROM vt{vt} WHERE rowid = {lo}"),
        VStmt::Delete { vt, lo, hi } => format!("DELETE FROM vt{vt} WHERE rowid BETWEEN {lo} AND {hi}"),
        VStmt::Update { .. } => unreachable!("rendered with its kind"),
        VStmt::Xform { vt, lo, hi, dx, dy } => format!("UPDATE vt{vt} SET _shape = geopoly_xform(_shape, 1, 0, 0, 1, {dx}, {dy}) WHERE rowid BETWEEN {lo} AND {hi}"),
        VStmt::Command { vt, cmd } => match cmd {
            Cmd::Optimize => format!("INSERT INTO vt{vt}(vt{vt}) VALUES ('optimize')"),
            Cmd::Rebuild => format!("INSERT INTO vt{vt}(vt{vt}) VALUES ('rebuild')"),
            Cmd::Integrity => format!("INSERT INTO vt{vt}(vt{vt}) VALUES ('integrity-check')"),
            // A value as text now and then (`sqlite3_value_numeric_type`
            // reads it as the integer), a key in upper case.
            Cmd::Config(k, n) => {
                let key = CONFIGS[*k % CONFIGS.len()].0;
                let key = if n % 5 == 3 { key.to_uppercase() } else { key.to_string() };
                let n = if n % 3 == 1 { format!("'{n}'") } else { n.to_string() };
                format!("INSERT INTO vt{vt}(vt{vt}, rank) VALUES ('{key}', {n})")
            },
            Cmd::Rank(a, b) => format!("INSERT INTO vt{vt}(vt{vt}, rank) VALUES ('rank', '{}')", bm25_spelled(*a, *b)),
            Cmd::DeleteAll => format!("INSERT INTO vt{vt}(vt{vt}) VALUES ('delete-all')"),
        },
        VStmt::Check(k) => format!("SELECT rtreecheck('vt{k}')"),
    }
}

/// `bm25(a.0, b.0)` in one of the forms `sqlite3Fts5ConfigParseRank`
/// reads (by `a` and `b`): signed, integer and decimal weights, spaces.
fn bm25_spelled(a: i64, b: i64) -> String {
    match (a + b) % 4 {
        0 => format!("bm25({a}.0, {b}.0)"),
        1 => format!("bm25({a}, +{b})"),
        2 => format!(" bm25 ( +{a}.00 ,{b}.0 ) "),
        _ => format!("bm25({a}.5, {b}.0, 1)"),
    }
}

/// The statement's SQL: `kind` that of the table it writes (its own for a
/// CREATE).
pub fn render(s: &VStmt, kind: &Kind) -> String {
    match s {
        VStmt::Fill { vt, start, count, mul, replace } => fill_sql(kind, *vt, *start, *count, *mul, *replace),
        VStmt::Insert { vt, .. } => {
            // An R-Tree's id is its first column, `k`.
            let cols: Vec<String> = match kind {
                Kind::Rtree { .. } => kind.columns(),
                _ => std::iter::once("rowid".to_string()).chain(kind.columns()).collect(),
            };
            sql(s, kind).replacen(&format!("INTO vt{vt} VALUES"), &format!("INTO vt{vt}({}) VALUES", cols.join(", ")), 1)
        },
        VStmt::Update { vt, id, sets } => {
            let names = kind.columns();
            let off = usize::from(matches!(kind, Kind::Rtree { .. }));
            let set: Vec<String> = sets
                .iter()
                .map(|(c, v)| {
                    let val = match v {
                        Value::Text(t) if *c == 0 && matches!(kind, Kind::Geopoly { .. }) => t.clone(),
                        v => lit(kind, v),
                    };
                    format!("{} = {val}", names[(c + off) % names.len()])
                })
                .collect();
            format!("UPDATE vt{vt} SET {} WHERE rowid = {id}", set.join(", "))
        },
        _ => sql(s, kind),
    }
}

pub fn cost(s: &VStmt) -> u64 {
    match s {
        VStmt::Create { kind, .. } => {
            2 + match kind {
                Kind::Rtree { dims, int, aux } => *dims as u64 + u64::from(*int) + *aux as u64,
                Kind::Geopoly { aux } => *aux as u64,
                Kind::Fts5 { cols, tok, prefix, detail, columnsize, contentless, unindexed, opts } => *cols as u64 + u64::from(*tok != Tok::Unicode61) + if let Tok::Categories(m) = tok { u64::from(m.count_ones()) } else { 0 } + u64::from(*prefix != 0) + u64::from(opts.count_ones()) + u64::from(*detail != Detail::Full) + u64::from(!columnsize) + u64::from(*contentless) + u64::from(*unindexed),
                Kind::Vocab { form, .. } => u64::from(*form),
            }
        },
        VStmt::Drop(_) => 1,
        VStmt::Insert { rows, replace, .. } => 2 + u64::from(*replace) + rows.iter().map(|r| 1 + r.id.unsigned_abs().min(1000) / 100 + r.values.iter().map(value_cost).sum::<u64>()).sum::<u64>(),
        VStmt::Fill { count, mul, replace, start, .. } => 2 + count.unsigned_abs() + mul.unsigned_abs() + u64::from(*replace) + start.unsigned_abs().min(1000) / 100,
        VStmt::Delete { lo, hi, .. } => 2 + (hi - lo).unsigned_abs().min(1000),
        VStmt::Update { sets, .. } => 2 + sets.iter().map(|(_, v)| 1 + value_cost(v)).sum::<u64>(),
        VStmt::Xform { lo, hi, dx, dy, .. } => 3 + (hi - lo).unsigned_abs().min(1000) + dx.unsigned_abs() + dy.unsigned_abs(),
        VStmt::Command { cmd, .. } => {
            2 + match cmd {
                Cmd::Config(k, n) => *k as u64 + n.unsigned_abs().min(100),
                Cmd::Rank(a, b) => (a + b) as u64,
                _ => 0,
            }
        },
        VStmt::Check(_) => 1,
    }
}

fn value_cost(v: &Value) -> u64 {
    match v {
        Value::Null => 0,
        Value::Int(_) | Value::Real(_) => 1,
        Value::Text(t) => 1 + t.len() as u64 / 4,
        Value::Blob(b) => 1 + b.len() as u64,
    }
}

/// Simpler statements: fewer rows, a smaller fill, a narrower range.
pub fn simpler(s: &VStmt) -> Vec<VStmt> {
    let mut out = Vec::new();
    match s {
        VStmt::Insert { vt, rows, replace } => {
            if *replace {
                out.push(VStmt::Insert { vt: *vt, rows: rows.clone(), replace: false });
            }
            if rows.len() > 1 {
                out.push(VStmt::Insert { vt: *vt, rows: rows[..rows.len() / 2].to_vec(), replace: *replace });
                out.push(VStmt::Insert { vt: *vt, rows: rows[1..].to_vec(), replace: *replace });
            }
        },
        VStmt::Fill { vt, start, count, mul, replace } => {
            if *count > 1 {
                out.push(VStmt::Fill { vt: *vt, start: *start, count: count / 2, mul: *mul, replace: *replace });
            }
            if *mul > 1 {
                out.push(VStmt::Fill { vt: *vt, start: *start, count: *count, mul: 1, replace: *replace });
            }
        },
        VStmt::Delete { vt, lo, hi } if lo < hi => out.push(VStmt::Delete { vt: *vt, lo: *lo, hi: *lo }),
        VStmt::Update { vt, id, sets } if sets.len() > 1 => {
            for k in 0..sets.len() {
                let mut s2 = sets.clone();
                s2.remove(k);
                out.push(VStmt::Update { vt: *vt, id: *id, sets: s2 });
            }
        },
        VStmt::Create { vt, kind: k @ Kind::Fts5 { .. } } => {
            let with = |f: &dyn Fn(&mut Kind)| {
                let mut c = k.clone();
                f(&mut c);
                c
            };
            for c in [
                with(&|c| if let Kind::Fts5 { tok, .. } = c { *tok = Tok::Unicode61 }),
                with(&|c| if let Kind::Fts5 { tok: Tok::Categories(m), .. } = c { *m = 0 }),
                with(&|c| if let Kind::Fts5 { prefix, .. } = c { *prefix = 0 }),
                with(&|c| if let Kind::Fts5 { detail, .. } = c { *detail = Detail::Full }),
                with(&|c| if let Kind::Fts5 { columnsize, .. } = c { *columnsize = true }),
                with(&|c| if let Kind::Fts5 { contentless, opts, .. } = c { *contentless = false; *opts &= !4 }),
                with(&|c| if let Kind::Fts5 { unindexed, opts, .. } = c { *unindexed = false; *opts &= !4 }),
                with(&|c| if let Kind::Fts5 { opts, .. } = c { *opts = 0 }),
            ] {
                if c != *k {
                    out.push(VStmt::Create { vt: *vt, kind: c });
                }
            }
        },
        _ => {},
    }
    out.retain(|x| cost(x) < cost(s));
    out
}

/// The statement against `vtabs` now, when it can stay: rows that clash
/// dropped, a fill made `OR REPLACE`.
pub fn repair(vtabs: &[VTable], s: &VStmt, open: bool) -> Option<VStmt> {
    if valid(vtabs, s, open) {
        return Some(s.clone());
    }
    let fixed = match s {
        VStmt::Insert { vt, rows, replace } if !*replace => {
            let t = vtabs.get(*vt)?;
            let mut seen = t.clone();
            let kept: Vec<VRow> = rows
                .iter()
                .filter(|r| {
                    let ok = row_ok(&t.kind, r) && !seen.has(r.id);
                    if ok {
                        seen.put((*r).clone());
                    }
                    ok
                })
                .cloned()
                .collect();
            VStmt::Insert { vt: *vt, rows: kept, replace: false }
        },
        VStmt::Fill { vt, start, count, mul, replace: false } => VStmt::Fill { vt: *vt, start: *start, count: *count, mul: *mul, replace: true },
        _ => return None,
    };
    (valid(vtabs, &fixed, open) && cost(&fixed) <= cost(s) + 1).then_some(fixed)
}

// ---------------------------------------------------------------- queries

/// What a query's CTE reads of a virtual table: its rowid and columns,
/// under constraints its `xBestIndex` takes.
#[derive(Clone, Debug, PartialEq)]
pub enum VQuery {
    /// `SELECT k, l0, h0, .. FROM vt{vt} WHERE ..`: each constraint a
    /// column, an operator of [`OPS`], a value.
    Rtree { vt: usize, cons: Vec<(usize, u8, Value)> },
    /// `SELECT rowid, p0, .., f(_shape) .. FROM vt{vt} [WHERE
    /// geopoly_overlap|within(_shape, shape)]`; `items` a mask of
    /// [`GEO_ITEMS`].
    /// `filter`: 0 or a term of [`GEO_FILTERS`] (`filter - 1`), which SQLite
    /// evaluates on every row the table gives.
    Geo { vt: usize, pred: Option<(bool, String)>, items: u32, rowid: Option<(i64, i64)>, filter: u8 },
    /// `SELECT rowid, s0, .., [highlight, snippet, bm25, rank] FROM vt{vt}
    /// WHERE <form> 'query'`: `form` 0 `vt MATCH`, 1 `vt = `, 2 the
    /// table-valued function, 3 `s{col} MATCH`, 4 no query at all; `top`
    /// an `ORDER BY rank LIMIT n`.
    /// `extra`: bit 0 a second MATCH (`AND vt MATCH '..'`), bit 1 a LIKE
    /// of a column (which a trigram table answers from its index), bit 2
    /// `rank MATCH 'bm25(..)'`. `top`'s bit 3: `ORDER BY rowid DESC` in
    /// place of `rank`.
    Fts { vt: usize, query: String, form: u8, items: u8, top: Option<u8>, rowid: Option<(i64, i64)>, extra: u8 },
    /// `SELECT nodeno, rtreenode(dims, data), length(data), rtreedepth
    /// of the root FROM vt{vt}_node`: an R-Tree's (or Geopoly table's)
    /// nodes, as its shadow table holds them.
    Node { vt: usize },
    /// `SELECT term, .. FROM vt{vt} [WHERE term <op> 'x']`, the operator
    /// of [`VOCAB_OPS`] (`BETWEEN 'x' AND 'xz'`).
    Vocab { vt: usize, from: Option<(u8, String)> },
}

const VOCAB_OPS: [&str; 6] = [">=", ">", "=", "<", "<=", "BETWEEN"];

/// An R-Tree constraint's operators: `!=` one its `xBestIndex` passes over
/// (SQLite filters by it).
const OPS: [&str; 8] = ["=", "<", "<=", ">", ">=", "BETWEEN", "IN", "!="];

/// [`VQuery::Geo`]'s items (bit 31 of the mask: the `geopoly_group_bbox`
/// query). Geopoly's functions give NULL for a value not a polygon, so
/// the invalid ones are as deterministic as the valid ones: a JSON ring of
/// exponents, spaces and a third value per vertex, rings too short, not
/// closed or with a leading zero (NULL), a big-endian blob (its floats
/// swapped on reading), a blob of the wrong length (NULL).
const GEO_ITEMS: [&str; 22] = [
    "geopoly_json(_shape)",
    "geopoly_area(_shape)",
    "geopoly_json(geopoly_bbox(_shape))",
    "geopoly_contains_point(_shape, 2, 3)",
    "geopoly_svg(_shape)",
    "geopoly_json(geopoly_ccw(_shape))",
    "geopoly_json(geopoly_xform(_shape, 2, 0, 0, 2, 1, -1))",
    "geopoly_overlap(_shape, geopoly_regular(4, 5, 7, 6))",
    "geopoly_within(_shape, '[[-12,-12],[30,-12],[30,30],[-12,30],[-12,-12]]')",
    "geopoly_overlap(_shape, '[[0,0],[10,0],[10,10],[0,10],[0,0]]')",
    "geopoly_contains_point(_shape, 5, 5)",
    "geopoly_contains_point(_shape, 0, 0.5)",
    "hex(geopoly_blob(_shape))",
    "geopoly_svg(_shape, 'fill=\"red\"', '', NULL)",
    "geopoly_json(geopoly_xform(_shape, 0.6, -0.8, 0.8, 0.6, 0, 0))",
    "geopoly_area(geopoly_json(_shape))",
    "geopoly_within(geopoly_regular(5, 5, 1, 4), _shape)",
    "geopoly_overlap(geopoly_bbox(_shape), geopoly_xform(_shape, 1, 0, 0, 1, 3, 2))",
    "geopoly_area(geopoly_regular(1, 2, 3, rowid % 1200))",
    "geopoly_json(' [ [0 , 0] , [1e1,0] , [1.5E+1,-2.5e-1] , [0,0.5] , [0,0] ] ')",
    "coalesce(geopoly_json('[[0,0],[1,1],[2,2]]'), geopoly_json('[[0,0],[01,1],[2,2],[0,0]]'), geopoly_json('[[0,0],[1,1],[2,2],[0,1]]'), geopoly_json(x'0100000300'), geopoly_json('[[0,0,9],[3,0],[3,3],[0,0]]'))",
    "geopoly_area(x'00000003000000000000000040800000000000000000000040800000')",
];

/// Terms of a Geopoly query's WHERE that no index takes (a function's
/// value compared, not the function itself), so SQLite computes them on
/// every row.
const GEO_FILTERS: [&str; 6] = [
    "geopoly_area(_shape) > 20",
    "geopoly_contains_point(_shape, 4, 5) > 0",
    "geopoly_overlap(_shape, geopoly_regular(5, 5, 8, 5)) > 0",
    "geopoly_within(_shape, '[[-12,-12],[30,-12],[30,30],[-12,30],[-12,-12]]') IS 2",
    "geopoly_area(geopoly_bbox(_shape)) >= abs(geopoly_area(_shape))",
    "geopoly_json(geopoly_ccw(_shape)) <> geopoly_json(_shape)",
];

/// [`VQuery::Geo`]'s `items` bit of the `geopoly_group_bbox` query.
const GEO_GROUP: u32 = 1 << 31;

/// A summary of the database's virtual tables, for queries
/// ([`crate::expr::Schema::of`] keeps it per thread).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VInfo {
    pub kind: Option<Kind>,
    /// Whether the table was written (made, or its rows changed) in a
    /// transaction still open: FTS5 may hold its writes in memory.
    pub pending: bool,
    pub rows: usize,
    /// Up to 6 values per column (rowid first).
    pub by_column: Vec<Vec<Value>>,
}

pub fn summary(vtabs: &[VTable], saved: Option<&[VTable]>) -> Vec<VInfo> {
    vtabs
        .iter()
        .enumerate()
        .map(|(k, t)| {
            let w = t.kind.width();
            let mut by_column = vec![Vec::new(); 1 + w];
            let step = (t.rows.len() / 6).max(1);
            for r in t.rows.iter().step_by(step).take(6) {
                by_column[0].push(Value::Int(r.id));
                for (c, v) in r.values.iter().enumerate() {
                    if !matches!(v, Value::Null) && !by_column[c + 1].contains(v) && !matches!(&t.kind, Kind::Geopoly { .. } if c == 0) && !t.kind.contentless() {
                        by_column[c + 1].push(v.clone());
                    }
                }
            }
            let rows = match &t.kind {
                // A trigram table's words are many tokens each.
                Kind::Vocab { of, .. } => vtabs.get(*of).map_or(0, |f| f.rows.len() * if matches!(f.kind, Kind::Fts5 { tok, .. } if tok.trigram()) { 100 } else { 40 } + 60),
                _ => t.rows.len(),
            };
            let pending = saved.is_some_and(|s| s.get(k) != Some(t));
            VInfo { kind: Some(t.kind.clone()), pending, rows, by_column }
        })
        .collect()
}

/// How many schema entries the virtual tables make (each, its shadow
/// tables and their indexes), and a bound of their shadow tables' pages.
pub fn shadows(info: &[VInfo]) -> (usize, usize) {
    info.iter()
        .map(|i| match &i.kind {
            Some(Kind::Fts5 { .. }) => (8, 20 + i.rows),
            Some(Kind::Rtree { .. } | Kind::Geopoly { .. }) => (5, 10 + i.rows / 4),
            _ => (1, 0),
        })
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

/// An FTS5 query of the syntax a table of `detail` and `tok` takes, of
/// `cols` columns.
pub fn draw_fts_query(g: &mut Source, cols: usize, detail: Detail, tok: Tok, depth: usize) -> String {
    let full = detail == Detail::Full;
    let word = |g: &mut Source| -> String {
        let w = if g.bool_with(0.15) { format!("{}{}", g.pick(&["t", "u", "v"]), g.int(0..13)) } else { g.pick(&WORDS).to_string() };
        if w.chars().count() < 3 && tok.trigram() {
            return "apple".into();
        }
        w
    };
    let leaf = |g: &mut Source| -> String {
        loop {
            match g.int(0..17) {
                0..=4 => return word(g),
                5 => {
                    let w = word(g);
                    let n = if tok.trigram() { 3 } else { g.int(1..=3) };
                    return format!("{}*", w.chars().take(n.max(1)).collect::<String>());
                },
                6 if full => return format!("\"{} {}\"", word(g), word(g)),
                7 if full => return format!("NEAR({} {}{})", word(g), word(g), if g.bool() { format!(", {}", g.int(0..=4)) } else { String::new() }),
                8 if full => return format!("^{}", word(g)),
                9 if full => return format!("{} + {}", word(g), word(g)),
                10 if detail != Detail::None => {
                    let c = g.index(cols);
                    return match g.int(0..3) {
                        0 => format!("s{c} : {}", word(g)),
                        1 if cols > 1 => format!("{{s0 s{}}} : {}", cols - 1, word(g)),
                        _ => format!("- s{c} : {}", word(g)),
                    };
                },
                11 if g.bool() => return format!("\"{}\"", word(g)),
                11 => return format!("{} {}*", word(g), word(g).chars().take(3).collect::<String>()),
                // NEAR of three phrases, one of two tokens.
                12 if full => return format!("NEAR(\"{} {}\" {} {}{})", word(g), word(g), word(g), word(g), if g.bool() { format!(", {}", g.int(0..=12)) } else { String::new() }),
                // A phrase whose last token is a prefix.
                13 if full => return format!("{} + {}*", word(g), word(g).chars().take(if tok.trigram() { 3 } else { g.int(1..=4) }).collect::<String>()),
                // A column filter of a phrase, NEAR or prefix.
                14 if detail != Detail::None => {
                    let f = match g.int(0..3) {
                        0 => format!("s{}", g.index(cols)),
                        1 if cols > 1 => format!("{{s{} s{}}}", cols - 2, cols - 1),
                        _ => format!("- {{s{}}}", g.index(cols)),
                    };
                    let p = match g.int(0..3) {
                        0 if full => format!("NEAR({} {}, {})", word(g), word(g), g.int(0..=3)),
                        1 if full => format!("\"{} {}\"", word(g), word(g)),
                        _ => format!("{}*", word(g).chars().take(if tok.trigram() { 3 } else { 2 }).collect::<String>()),
                    };
                    return format!("{f} : {p}");
                },
                // An initial phrase.
                15 if full => return format!("^\"{} {}\"", word(g), word(g)),
                // A quoted string with an escaped quote, two tokens.
                16 if full => return format!("\"{}\"\"{}\"", word(g), word(g)),
                _ => {},
            }
        }
    };
    if depth == 0 || g.bool_with(0.45) {
        return leaf(g);
    }
    let a = draw_fts_query(g, cols, detail, tok, depth - 1);
    let b = draw_fts_query(g, cols, detail, tok, depth - 1);
    match g.int(0..5) {
        0 => format!("{a} AND {b}"),
        1 => format!("{a} OR {b}"),
        2 => format!("{a} NOT {b}"),
        // Two phrases side by side are an AND; parenthesized ones are not
        // phrases ("syntax error near \"(\"").
        3 => format!("({a}) AND ({b})"),
        _ if detail != Detail::None => format!("s{} : ({a} OR {b})", g.index(cols)),
        _ => format!("({a} OR {b})"),
    }
}

/// Whether a table of `detail` takes the query's syntax (a query drawn for
/// another table, which a mutation or shrink changed): no phrase of more
/// than one token, NEAR, `^` or `+` but with detail=full ("phrase queries
/// are not supported (detail!=full)"), no column filter with detail=none.
fn syntax_ok(query: &str, detail: Detail) -> bool {
    let phrase = query.contains("NEAR(") || query.contains('^') || query.contains(" + ") || query.split('"').skip(1).step_by(2).any(|p| p.contains(' ') || p.is_empty()) || query.contains("\"\"");
    match detail {
        Detail::Full => true,
        Detail::Column => !phrase,
        Detail::None => !phrase && !query.contains(':'),
    }
}

/// Whether every column a query's text names (`s1 : ..`, `{s0 s2} : ..`,
/// `- {s1} : ..`) is one of a table of `cols` columns: a query drawn for a
/// wider table (a seed's queries on a fresh case, a mutation or a shrink
/// that changed the table) fails with "no such column: s2" otherwise. No
/// word of the queries is an `s` and digits.
fn columns_within(query: &str, cols: usize) -> bool {
    let b = query.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    (0..b.len()).filter(|&i| b[i] == b's' && (i == 0 || !word(b[i - 1]))).all(|i| {
        let end = (i + 1..b.len()).find(|&j| !b[j].is_ascii_digit()).unwrap_or(b.len());
        end == i + 1 || (end < b.len() && word(b[end])) || query[i + 1..end].parse::<usize>().is_ok_and(|c| c < cols)
    })
}

impl VQuery {
    pub fn vt(&self) -> usize {
        match self {
            VQuery::Rtree { vt, .. } | VQuery::Geo { vt, .. } | VQuery::Fts { vt, .. } | VQuery::Vocab { vt, .. } | VQuery::Node { vt } => *vt,
        }
    }

    /// Whether it reads a table of this kind.
    fn fits(&self, kind: &Kind) -> bool {
        matches!((self, kind), (VQuery::Rtree { .. }, Kind::Rtree { .. }) | (VQuery::Geo { .. }, Kind::Geopoly { .. }) | (VQuery::Fts { .. }, Kind::Fts5 { .. }) | (VQuery::Vocab { .. }, Kind::Vocab { .. }) | (VQuery::Node { .. }, Kind::Rtree { .. } | Kind::Geopoly { .. }))
    }

    pub fn columns(&self, kind: &Kind) -> Vec<String> {
        let vt = self.vt();
        match (self, kind) {
            (VQuery::Rtree { .. }, k) => k.columns(),
            // `geopoly_group_bbox` over the rows: one row (no `total` of
            // areas, whose sum depends on the order the rows are read in).
            (VQuery::Geo { items, .. }, _) if items & GEO_GROUP != 0 => vec!["geopoly_json(geopoly_group_bbox(_shape))".to_string(), "count(*)".to_string()],
            (VQuery::Node { .. }, k) => {
                let dims = if let Kind::Rtree { dims, .. } = k { *dims } else { 2 };
                vec!["nodeno".to_string(), format!("rtreenode({dims}, data)"), "length(data)".to_string(), "CASE nodeno WHEN 1 THEN rtreedepth(data) END".to_string()]
            },
            // `+rowid`: an equality of a Geopoly table's rowid with a value
            // not an integer (an outer `USING (c0)`) matches the row of its
            // integer part (`rowid = 6.5` gives rowid 6, trunk 185ffd27064b;
            // bugs/sqlite/sqlite-geopoly-rowid-eq-nonint).
            (VQuery::Geo { items, .. }, k) => {
                let mut c = vec!["+rowid".to_string()];
                c.extend(k.columns().into_iter().skip(1));
                c.extend((0..GEO_ITEMS.len()).filter(|i| items & (1 << i) != 0).map(|i| GEO_ITEMS[i].to_string()));
                c
            },
            (VQuery::Fts { items, form, .. }, k @ Kind::Fts5 { cols, detail, unindexed, tok, .. }) => {
                // snippet of every column (-1) with an UNINDEXED one and
                // detail column or none fails ("database disk image is
                // malformed", 3.51.0 and trunk 5bd093e261a7;
                // bugs/sqlite/sqlite-fts5-unindexed-detail-none-instances).
                let every = !(*unindexed && *cols > 1 && *detail != Detail::Full);
                // With a MATCH, `+rowid`: an equality of the rowid with a
                // value not an integer (an outer `USING (c0)`) gives every
                // row the MATCH gives (trunk 5bd093e261a7 and the vendored
                // SQLite, not 3.51.0; bugs/sqlite/
                // sqlite-fts5-match-rowid-eq-nonint).
                let mut c = vec![if *form == 4 { "rowid" } else { "+rowid" }.to_string()];
                c.extend(k.columns());
                // A trigram table's UNINDEXED column as `+s1`: a LIKE or GLOB
                // of it in the outer query, pushed down to the table, would
                // be answered from the index, which holds none of its tokens
                // (no rows; the known SQLite bug
                // sqlite-fts5-trigram-like-unindexed, run 20261022-0005).
                if *unindexed && *cols > 1 && tok.trigram() {
                    c[*cols] = format!("+{}", c[*cols]);
                }
                // The auxiliary functions only over a MATCH: with `vt = 'q'`
                // (or the table-valued function's argument, the same
                // constraint) an outer WHERE term of one fails ("no such
                // cursor: 0", 3.51.0 and trunk 5bd093e261a7;
                // bugs/sqlite/sqlite-fts5-eq-constprop-aux-cursor).
                if *form == 0 || *form == 3 {
                    if items & 1 != 0 {
                        c.push(format!("highlight(vt{vt}, {}, '[', ']')", *items as usize % cols));
                    }
                    if items & 2 != 0 {
                        c.push(format!("snippet(vt{vt}, {}, '<', '>', '..', {})", if items & 16 != 0 && every { -1 } else { 0 }, 2 + (items >> 5) % 4));
                    }
                    if items & 4 != 0 {
                        c.push(format!("round(bm25(vt{vt}{}), 6)", if items & 64 != 0 { ", 2.0, 0.5" } else { "" }));
                    }
                    if items & 8 != 0 {
                        c.push("round(rank, 6)".to_string());
                    }
                    if items & 128 != 0 && k.locale() {
                        c.push(format!("fts5_get_locale(vt{vt}, {})", (*items as usize >> 2) % cols));
                    }
                }
                c
            },
            (_, k) => k.columns(),
        }
    }

    /// `SELECT .. FROM ..`, `noindex` with its constraints unusable by the
    /// virtual table (`+col`): what the virtual table returns unfiltered,
    /// SQLite filters.
    pub fn sql(&self, kind: &Kind, noindex: bool) -> String {
        self.sql_in(kind, noindex, false)
    }

    /// [`VQuery::sql`] of a table with FTS5 writes pending in an open
    /// transaction (`pending`, [`VInfo::pending`]): then, on a
    /// `tokendata=1` table with a prefix index, its full-text query without
    /// prefix terms (`v*` read as `v`). A prefix query opened before a term
    /// query there reads freed memory, the term query flushing the pending
    /// terms under it ("database disk image is malformed"; the known SQLite
    /// bug sqlite-fts5-tokendata-prefix-corrupt, run 20261022-0005).
    pub fn sql_in(&self, kind: &Kind, noindex: bool, pending: bool) -> String {
        let vt = self.vt();
        let unprefixed = pending && matches!(kind, Kind::Fts5 { prefix, opts, .. } if *prefix != 0 && opts & 1 != 0);
        let items: Vec<String> = self.columns(kind).iter().enumerate().map(|(k, c)| format!("{c} AS c{k}")).collect();
        let plus = if noindex { "+" } else { "" };
        let rowid_range = |w: &mut Vec<String>, r: &Option<(i64, i64)>| {
            if let Some((lo, hi)) = r {
                w.push(if lo == hi { format!("{plus}rowid = {lo}") } else { format!("{plus}rowid BETWEEN {lo} AND {hi}") });
            }
        };
        let mut wh = Vec::new();
        let mut from = if let VQuery::Node { .. } = self { format!("vt{vt}_node") } else { format!("vt{vt}") };
        let mut tail = String::new();
        match self {
            VQuery::Rtree { cons, .. } => {
                let names = kind.columns();
                for (c, op, v) in cons {
                    let n = &names[*c % names.len()];
                    let op = OPS[usize::from(*op) % OPS.len()];
                    if op == "BETWEEN" {
                        wh.push(format!("{plus}{n} BETWEEN {} AND {}", literal(v), literal(&Value::Real(num(v) + 5.0))));
                    } else if op == "IN" {
                        wh.push(format!("{plus}{n} IN ({}, {}, {})", literal(v), literal(&Value::Real(num(v) + 1.0)), literal(&Value::Real(num(v) - 2.5))));
                    } else {
                        wh.push(format!("{plus}{n} {op} {}", literal(v)));
                    }
                }
            },
            VQuery::Geo { pred, rowid, filter, .. } => {
                if let Some((within, shape)) = pred {
                    wh.push(format!("geopoly_{}({plus}_shape, {shape})", if *within { "within" } else { "overlap" }));
                }
                if *filter != 0 {
                    wh.push(GEO_FILTERS[usize::from(*filter - 1) % GEO_FILTERS.len()].to_string());
                }
                rowid_range(&mut wh, rowid);
            },
            VQuery::Fts { query, form, top, rowid, items, extra, .. } => {
                let q = literal(&Value::Text(if unprefixed { query.replace('*', "") } else { query.clone() }));
                match form {
                    0 => wh.push(format!("vt{vt} MATCH {q}")),
                    1 => wh.push(format!("vt{vt} = {q}")),
                    2 => from = format!("vt{vt}({q})"),
                    3 => wh.push(format!("s0 MATCH {q}")),
                    _ => {},
                }
                rowid_range(&mut wh, rowid);
                let matched = *form != 4;
                // A second MATCH and `rank MATCH` only with the MATCH forms:
                // after `vt = 'q'` SQLite may read `vt MATCH 'r'` as the
                // function on 'q' ("unable to use function MATCH in the
                // requested context"; see sqlite-fts5-eq-constprop-aux-cursor).
                let match_form = *form == 0 || *form == 3;
                if extra & 1 != 0 && match_form {
                    wh.push(format!("vt{vt} MATCH 'zeta OR apple OR t{}'", extra >> 4));
                }
                if extra & 2 != 0 {
                    let w = WORDS[usize::from(extra >> 3) % WORDS.len()];
                    // Written the same in the NOT INDEXED variant when the
                    // query reads an auxiliary function or rank: a trigram
                    // table answers the LIKE from its index as part of the
                    // full-text query, which highlight, snippet, bm25 and
                    // rank then read (a `+s0 LIKE` is no part of it).
                    let lplus = if items & 143 != 0 { "" } else { plus };
                    wh.push(format!("{lplus}s0 {}", trigram_pattern(w, extra >> 3)));
                }
                if extra & 4 != 0 && match_form {
                    wh.push(format!("rank MATCH '{}'", bm25_spelled(1 + i64::from(extra >> 4), 1)));
                }
                if let Some(n) = top {
                    if n & 8 != 0 {
                        tail = format!(" ORDER BY rowid DESC LIMIT {}", 1 + n % 8);
                    } else if matched {
                        tail = format!(" ORDER BY rank{} LIMIT {}", if n & 16 != 0 { " DESC" } else { "" }, 1 + n % 8);
                    }
                } else if (*form == 0 || *form == 3) && items & 135 != 0 {
                    // An auxiliary function (highlight, snippet, bm25) only
                    // where its cursor is: flattened into an aggregate
                    // query, a reference from a correlated subquery reads
                    // it as an aggregate's column ("unable to use function
                    // bm25 in the requested context"), and SQLite never
                    // flattens a query with a LIMIT.
                    tail = " LIMIT -1".to_string();
                }
            },
            VQuery::Node { .. } => {},
            VQuery::Vocab { from: f, .. } => {
                if let Some((op, t)) = f {
                    let op = VOCAB_OPS[usize::from(*op) % VOCAB_OPS.len()];
                    let v = literal(&Value::Text(t.clone()));
                    wh.push(if op == "BETWEEN" { format!("{plus}term BETWEEN {v} AND {}", literal(&Value::Text(format!("{t}z")))) } else { format!("{plus}term {op} {v}") });
                }
            },
        }
        let wh = if wh.is_empty() { String::new() } else { format!(" WHERE {}", wh.join(" AND ")) };
        format!("SELECT {} FROM {from}{wh}{tail}", items.join(", "))
    }

    pub fn cost(&self) -> u64 {
        1 + match self {
            VQuery::Rtree { cons, .. } => cons.iter().map(|(c, o, v)| 1 + *c as u64 + u64::from(*o) + value_cost(v)).sum::<u64>(),
            VQuery::Geo { pred, items, rowid, filter, .. } => pred.as_ref().map_or(0, |(w, s)| 2 + u64::from(*w) + s.len() as u64 / 8) + u64::from(items.count_ones()) + u64::from(rowid.is_some()) + u64::from(*filter != 0),
            VQuery::Fts { query, form, items, top, rowid, extra, .. } => query.len() as u64 + u64::from(*form) + u64::from(items.count_ones()) + u64::from(top.is_some()) + u64::from(rowid.is_some()) + u64::from(extra.count_ones()),
            VQuery::Node { .. } => 1,
            VQuery::Vocab { from, .. } => from.as_ref().map_or(0, |(op, _)| 1 + u64::from(*op)),
        }
    }

    /// Whether it reads a table that is there, of its kind, as it may.
    pub fn valid(&self, info: &[VInfo]) -> bool {
        let Some(Some(kind)) = info.get(self.vt()).map(|i| i.kind.as_ref()) else { return false };
        self.fits(kind)
            && match (self, kind) {
                (VQuery::Rtree { cons, .. }, k) => cons.iter().all(|(c, o, v)| *c < k.columns().len() && usize::from(*o) < OPS.len() && (matches!(v, Value::Int(_) | Value::Real(_) | Value::Null) || *v == Value::Text("k".into()))),
                (VQuery::Geo { filter, .. }, _) => usize::from(*filter) <= GEO_FILTERS.len(),
                (VQuery::Fts { form, query, .. }, Kind::Fts5 { detail, cols, .. }) => *form <= 4 && (*form == 4 || !query.is_empty()) && !(*form == 3 && *detail == Detail::None) && !query.contains('\'') && syntax_ok(query, *detail) && columns_within(query, *cols),
                _ => true,
            }
    }

    /// About how many rows it gives (all of the table's), values its
    /// columns hold.
    pub fn info(&self, info: &[VInfo]) -> Option<(usize, Vec<Vec<Value>>, usize)> {
        if !self.valid(info) {
            return None;
        }
        let i = &info[self.vt()];
        let kind = i.kind.as_ref()?;
        let width = self.columns(kind).len();
        let mut by_column = vec![Vec::new(); width];
        match self {
            // An R-Tree's columns are its id and values; a Geopoly
            // query's the rowid and the auxiliary columns (the shape left
            // out); an FTS5 query's the rowid and columns.
            VQuery::Rtree { .. } | VQuery::Fts { .. } => {
                for (c, v) in by_column.iter_mut().zip(&i.by_column) {
                    *c = v.clone();
                }
            },
            VQuery::Geo { items, .. } if items & GEO_GROUP != 0 => {},
            VQuery::Geo { .. } => {
                by_column[0] = i.by_column[0].clone();
                for (c, v) in by_column.iter_mut().skip(1).zip(i.by_column.iter().skip(2)) {
                    *c = v.clone();
                }
            },
            VQuery::Vocab { .. } => by_column[0] = WORDS[..6].iter().map(|w| Value::Text(w.to_string())).collect(),
            VQuery::Node { .. } => by_column[0] = (1..=3).map(Value::Int).collect(),
        }
        let rows = match self {
            VQuery::Geo { items, .. } if items & GEO_GROUP != 0 => 1,
            VQuery::Node { .. } => i.rows + 2,
            _ => i.rows,
        };
        Some((rows.max(1), by_column, width))
    }

    /// Simpler forms: fewer constraints or items.
    pub fn simpler(&self) -> Vec<VQuery> {
        let mut out = Vec::new();
        match self {
            VQuery::Rtree { vt, cons } => {
                for k in 0..cons.len() {
                    let mut c = cons.clone();
                    c.remove(k);
                    out.push(VQuery::Rtree { vt: *vt, cons: c });
                }
            },
            VQuery::Geo { vt, pred, items, rowid, filter } => {
                if pred.is_some() {
                    out.push(VQuery::Geo { vt: *vt, pred: None, items: *items, rowid: *rowid, filter: *filter });
                }
                if *items != 0 {
                    out.push(VQuery::Geo { vt: *vt, pred: pred.clone(), items: 0, rowid: *rowid, filter: *filter });
                }
                if rowid.is_some() {
                    out.push(VQuery::Geo { vt: *vt, pred: pred.clone(), items: *items, rowid: None, filter: *filter });
                }
                if *filter != 0 {
                    out.push(VQuery::Geo { vt: *vt, pred: pred.clone(), items: *items, rowid: *rowid, filter: 0 });
                }
            },
            VQuery::Fts { vt, query, form, items, top, rowid, extra } => {
                let with = |f: &dyn Fn(&mut VQuery)| {
                    let mut c = self.clone();
                    f(&mut c);
                    c
                };
                if *items != 0 {
                    out.push(with(&|c| if let VQuery::Fts { items, .. } = c { *items = 0 }));
                }
                if *extra != 0 {
                    out.push(with(&|c| if let VQuery::Fts { extra, .. } = c { *extra = 0 }));
                }
                if top.is_some() {
                    out.push(with(&|c| if let VQuery::Fts { top, .. } = c { *top = None }));
                }
                if rowid.is_some() {
                    out.push(with(&|c| if let VQuery::Fts { rowid, .. } = c { *rowid = None }));
                }
                if *form != 0 && *form != 4 {
                    out.push(with(&|c| if let VQuery::Fts { form, .. } = c { *form = 0 }));
                }
                if query.len() > 5 {
                    out.push(VQuery::Fts { vt: *vt, query: "apple".into(), form: *form, items: *items, top: *top, rowid: *rowid, extra: *extra });
                }
            },
            VQuery::Vocab { vt, from: Some(_) } => out.push(VQuery::Vocab { vt: *vt, from: None }),
            VQuery::Vocab { .. } | VQuery::Node { .. } => {},
        }
        out.retain(|s| s.cost() < self.cost());
        out
    }
}

/// A query of one of the virtual tables `info` lists.
pub fn draw_query(g: &mut Source, info: &[VInfo]) -> Option<VQuery> {
    let vt = g.index(info.len());
    let i = &info[vt];
    let kind = i.kind.as_ref()?;
    let rowid = |g: &mut Source| -> Option<(i64, i64)> {
        if !g.bool_with(0.2) || i.by_column[0].is_empty() {
            return None;
        }
        let a = match g.pick(&i.by_column[0]) {
            Value::Int(a) => *a,
            _ => 0,
        };
        Some((a, a + *g.pick(&[0i64, 0, 3, 50])))
    };
    Some(match kind {
        Kind::Rtree { .. } | Kind::Geopoly { .. } if g.bool_with(0.15) => VQuery::Node { vt },
        Kind::Rtree { .. } => {
            let mut cons = Vec::new();
            for _ in 0..*g.weighted(&[(1, 0usize), (3, 1), (3, 2), (2, 3), (1, 4), (1, 6)]) {
                let c = g.index(i.by_column.len());
                let v = match (i.by_column[c].is_empty(), g.int(0..4)) {
                    (false, 0..=2) => g.pick(&i.by_column[c]).clone(),
                    _ => Value::Int(g.int(-20i64..=40)),
                };
                let v = match v {
                    // Now and then half a unit off: between two of an
                    // rtree_i32 table's values, or of a float tree's.
                    Value::Int(i) if g.bool_with(0.15) => Value::Real(i as f64 + 0.5),
                    Value::Real(x) if g.bool_with(0.15) => Value::Real(x - 0.25),
                    // An id or coordinate as a REAL of an integer (a
                    // rowid lookup takes it as the integer).
                    Value::Int(i) if g.bool_with(0.1) => Value::Real(i as f64),
                    // Values the R-Tree takes as no number: a text not
                    // numeric (below `<` and `<=` everything, above `>`,
                    // `>=` and `=` nothing, as SQLite compares a number
                    // with a text) and NULL; integers past 2^48, of
                    // which `<` and `>` are `<=` and `>=` to the tree.
                    _ if g.bool_with(0.08) => match g.int(0..4) {
                        0 => Value::Null,
                        1 => Value::Text("k".into()),
                        _ => Value::Int(*g.pick(&[1i64 << 48, (1i64 << 48) + 1, -(1i64 << 48), i64::MAX])),
                    },
                    Value::Int(_) | Value::Real(_) => v,
                    _ => Value::Int(g.int(-20i64..=40)),
                };
                cons.push((c, g.int(0..OPS.len() as u8), v));
            }
            VQuery::Rtree { vt, cons }
        },
        Kind::Geopoly { .. } => {
            // A third of the shapes large, holding or overlapping most
            // rows, so that a query gives rows to compute its items on.
            let pred = g.bool_with(0.75).then(|| (g.bool_with(0.35), if g.bool_with(0.35) { format!("geopoly_regular({}, {}, {}, {})", g.int(0i64..=10), g.int(0i64..=10), g.int(12i64..=30), g.int(4i64..=12)) } else { shape_sql(g) }));
            let filter = if g.bool_with(0.4) { 1 + g.int(0..GEO_FILTERS.len() as u8) } else { 0 };
            VQuery::Geo { vt, pred, items: g.int(0..1u32 << GEO_ITEMS.len()) | if g.bool_with(0.25) { GEO_GROUP } else { 0 }, rowid: rowid(g), filter }
        },
        Kind::Fts5 { cols, tok, detail, .. } => {
            let form = *g.weighted(&[(6, 0u8), (2, 1), (2, 2), (1, 3), (1, 4)]);
            let form = if form == 3 && *detail == Detail::None { 0 } else { form };
            let depth = *g.weighted(&[(1, 1), (3, 2), (1, 3)]);
            let query = if form == 4 { String::new() } else { draw_fts_query(g, *cols, *detail, *tok, depth) };
            VQuery::Fts { vt, query, form, items: if g.bool_with(0.75) { g.int(1..=255u8) } else { 0 }, top: g.bool_with(0.25).then(|| g.int(0..32u8)), rowid: rowid(g), extra: if g.bool_with(0.3) { g.int(0..=255u8) } else { 0 } }
        },
        Kind::Vocab { .. } => VQuery::Vocab {
            vt,
            from: g.bool_with(0.6).then(|| {
                let w = g.pick(&WORDS);
                (g.int(0..VOCAB_OPS.len() as u8), if g.bool() { w.to_string() } else { w.chars().take(2).collect() })
            }),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two known FTS5 bugs a virtual table's query keeps from: a
    /// trigram table's UNINDEXED column is read as `+s1`, so an outer GLOB
    /// of it is not pushed down to the table (which answers it with no
    /// rows, sqlite-fts5-trigram-like-unindexed); a `tokendata=1` table
    /// with a prefix index and writes pending in an open transaction is
    /// queried without prefix terms (sqlite-fts5-tokendata-prefix-corrupt).
    #[test]
    fn fts5_queries_keep_from_the_known_bugs() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        db.execute_batch("CREATE VIRTUAL TABLE vt0 USING fts5(s0, s1 UNINDEXED, tokenize=trigram); INSERT INTO vt0 VALUES ('apple', 'CONDITIONAL');").unwrap();
        let tri = Kind::Fts5 { cols: 2, tok: Tok::Trigram, prefix: 0, detail: Detail::Full, columnsize: true, contentless: false, unindexed: true, opts: 0 };
        let q = VQuery::Fts { vt: 0, query: String::new(), form: 4, items: 0, top: None, rowid: None, extra: 0 };
        let body = q.sql(&tri, false);
        assert!(body.contains("+s1 AS c2"), "{body}");
        assert_eq!(count(&format!("WITH x AS ({body}) SELECT count(*) FROM x WHERE c2 GLOB '*CONDITIO*'")), 1);
        assert_eq!(count("WITH x AS (SELECT s1 AS c2 FROM vt0) SELECT count(*) FROM x WHERE c2 GLOB '*CONDITIO*'"), 0);
        db.execute_batch("CREATE VIRTUAL TABLE vt1 USING fts5(s0, tokendata=1, prefix=1); INSERT INTO vt1 VALUES ('vv aa'); BEGIN; INSERT INTO vt1 VALUES ('vv aa');").unwrap();
        let td = Kind::Fts5 { cols: 1, tok: Tok::Unicode61, prefix: 2, detail: Detail::Full, columnsize: true, contentless: false, unindexed: false, opts: 1 };
        let q = VQuery::Fts { vt: 1, query: "v* AND aa".into(), form: 0, items: 0, top: None, rowid: None, extra: 0 };
        assert!(q.sql(&td, false).contains("'v* AND aa'"));
        let body = q.sql_in(&td, false, true);
        assert!(body.contains("'v AND aa'"), "{body}");
        assert_eq!(count(&format!("WITH x AS ({body}) SELECT count(*) FROM x")), 0);
        let vv = VQuery::Fts { vt: 1, query: "vv* AND aa".into(), form: 0, items: 0, top: None, rowid: None, extra: 0 };
        assert_eq!(count(&format!("WITH x AS ({}) SELECT count(*) FROM x", vv.sql_in(&td, false, true))), 2);
    }

    /// An fts5vocab table's `term` range in a UTF-16 database takes terms
    /// in their UTF-8 order, its own constraint's or one a query around it
    /// pushes down (the SQLite bug [`vocab_ok`] keeps the cases from).
    #[test]
    fn vocab_ranges_differ_in_utf16_databases() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA encoding = 'UTF-16le'; CREATE VIRTUAL TABLE ft USING fts5(s0); INSERT INTO ft VALUES ('apple ĳs'); CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'row');").unwrap();
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count("SELECT count(*) FROM v WHERE term < 'b'"), 1);
        assert_eq!(count("SELECT count(*) FROM v WHERE +term < 'b'"), 2);
        assert_eq!(count("WITH x AS (SELECT term AS c0 FROM v) SELECT count(*) FROM x WHERE c0 < 'b'"), 1);
    }

    /// A query names only columns its table has ([`columns_within`]):
    /// every drawn query on its own table, on a table of one column fewer
    /// only when it names none of the last (`{s0 s2} : t12` on two).
    #[test]
    fn fts_queries_name_their_tables_columns() {
        assert!(columns_within("s1 : (- {s1} : di* OR {s0 s2} : lemon AND \"chess\")", 3));
        assert!(!columns_within("s1 : (- {s1} : di* OR {s0 s2} : lemon AND \"chess\")", 2));
        assert!(!columns_within("{s0 s2} : t12 OR \"chess\"", 1));
        assert!(!columns_within("s1 : sky", 1));
        assert!(columns_within("sky OR sing* OR t12 OR sensibiliti", 1));
        for seed in 0..3000u64 {
            let mut g = Source::new(seed, 50);
            let cols = 1 + g.index(4);
            let q = draw_fts_query(&mut g, cols, Detail::Full, Tok::Unicode61, 3);
            assert!(columns_within(&q, cols), "{q} ({cols} columns)");
            assert_eq!(columns_within(&q, cols - 1), !q.contains(&format!("s{}", cols - 1)), "{q} ({cols} columns)");
        }
    }

    /// Every spelling of an FTS5 table's CREATE ([`Spelling`],
    /// [`Tok::spelled`]) is one SQLite takes: drawn tables, each under four
    /// numbers, and every tokenizer in each of its forms.
    #[test]
    fn fts5_spellings_create() {
        let mut n = 0;
        for seed in 0..1500u64 {
            let kind = draw_kind(&mut Source::new(seed, 50), &[]);
            if !kind.fts() || !valid(&[], &VStmt::Create { vt: 0, kind: kind.clone() }, false) {
                continue;
            }
            // A connection of its own (a contentless_unindexed=1 table's
            // DROP leaves its content table: bugs/sqlite/sqlite-fts5-contentless-unindexed-drop-rename).
            let db = rusqlite::Connection::open_in_memory().unwrap();
            for k in 0..4 {
                let sql = kind.create_sql(k);
                db.execute_batch(&sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
                db.execute_batch(&format!("INSERT INTO vt{k}(rowid, s0) VALUES (1, 'apple café x-y.z'); SELECT * FROM vt{k} WHERE vt{k} MATCH 'apple'")).unwrap_or_else(|e| panic!("{sql}: {e}"));
                n += 1;
            }
        }
        assert!(n > 1000, "{n}");
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for tok in Tok::ALL {
            for k in 0..4 {
                if let Some(t) = tok.spelled(k) {
                    db.execute_batch(&format!("CREATE VIRTUAL TABLE t USING fts5(a, tokenize='{t}'); DROP TABLE t")).unwrap_or_else(|e| panic!("{t}: {e}"));
                }
            }
        }
    }

    /// The fill's SQL computes the model's texts ([`fill_text`]), with the
    /// words it adds.
    #[test]
    fn fills_compute_the_model_texts() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for more in [&UNICODE_WORDS[..], &PORTER_WORDS[..]] {
            for c in 0..3 {
                for mul in [1, 7, 13] {
                    let sql = format!("WITH RECURSIVE r(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM r WHERE i < 99) SELECT group_concat({}, char(10)) FROM r", fill_text_sql(mul, c, more));
                    let got: String = db.query_row(&sql, [], |r| r.get(0)).unwrap();
                    let want: Vec<String> = (0..100).map(|i| fill_text(i, mul, c, more)).collect();
                    assert_eq!(got, want.join("\n"), "{c} {mul}");
                }
            }
        }
    }
}
