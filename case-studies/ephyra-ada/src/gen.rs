//! Cases drawn from a grammar of URLs: [`cases`].
//!
//! Every string is a valid input to a URL parser, so a case needs no
//! predicate; what matters is reaching the parser's states. The grammar
//! builds an input from a scheme, slashes, credentials, a host of each kind
//! the standard parses (domains with Unicode and punycode labels, IPv4 in
//! every radix and part count, IPv6 with compression and embedded IPv4,
//! opaque hosts), a port, path segments with dot segments and drive letters,
//! a query and a fragment, each with characters the component must
//! percent-encode, then adds the noise the parser strips (C0 controls and
//! spaces around it, tabs and newlines inside it). Relative inputs come
//! with a base, and setter calls follow with values drawn for the setter.
//!
//! Shrinking works on the final strings: drop a setter, drop the base
//! (keeping an input that does not need it), then delete runs of
//! characters (halves down to single characters), then replace a character
//! by `a`. Each candidate lowers the measure (setters, base present, total
//! characters, characters other than `a`), lexicographically, so chains
//! settle. Mutation replaces a run of an input with a freshly drawn
//! fragment, adds or replaces a setter call, or adds noise.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};

use crate::{Case, Setter};

/// `n` in `0..=max`, smaller more often: a fair coin per step.
pub(crate) fn few(g: &mut Source, max: usize) -> usize {
    let mut n = 0;
    while n < max && g.bool() {
        n += 1;
    }
    n
}

/// One of `options`, weighted by the first field.
fn weighted<'a, T>(g: &mut Source, options: &'a [(u32, T)]) -> &'a T {
    let total: u32 = options.iter().map(|o| o.0).sum();
    let mut at = g.int(0..total);
    for (w, t) in options {
        if at < *w {
            return t;
        }
        at -= w;
    }
    &options[options.len() - 1].1
}

pub(crate) fn pick_str(g: &mut Source, options: &[&str]) -> String {
    (*g.pick(options)).to_owned()
}

/// Randomly upper- or lower-case ASCII letters, now and then.
fn recase(g: &mut Source, s: &str) -> String {
    if g.bool_with(0.75) {
        return s.to_owned();
    }
    s.chars()
        .map(|c| if g.bool() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
        .collect()
}

/// Code points whose IDNA mapping and punycode grow the most per input
/// byte, measured with tr46 over every code point: `㍿` (3 bytes) becomes
/// the 17-byte label `xn--6oqv20b1zgzxr`, `㌖` 16 bytes, `¼` (2 bytes) 10.
/// Setters bound how much a value can grow the href; these test the bound.
const EXPANDING: &[char] =
    &['\u{337F}', '\u{3316}', '\u{321E}', '\u{3319}', '\u{332B}', '\u{33AF}', '¼', '½', '\u{3300}'];

/// Non-ASCII characters the standard treats specially somewhere: IDNA
/// mappings (full-width forms, the ideographic full stop and other dots,
/// `ß`, final sigma, soft hyphen, ZWJ/ZWNJ, combining marks, compatibility
/// characters, the expanding ones above), RTL letters, characters that map
/// to forbidden ASCII, and the replacement character, byte-order mark and
/// noncharacters.
const UNICODE: &[char] = &[
    '\u{337F}',
    '\u{3316}',
    '¼',
    'é',
    'ß',
    'ς',
    'σ',
    'Σ',
    'ı',
    'İ',
    'ﬀ',
    'ﬁ',
    'Ⅻ',
    '①',
    '™',
    'ª',
    'µ',
    'ǅ',
    '\u{00AD}',
    '\u{200C}',
    '\u{200D}',
    '\u{0301}',
    '\u{0308}',
    '\u{034F}',
    '\u{180B}',
    '\u{FE0F}',
    '\u{3002}',
    '\u{FF0E}',
    '\u{FF61}',
    '\u{2024}',
    'Ａ',
    'ｚ',
    '０',
    '９',
    '\u{FF05}',
    '\u{FF0F}',
    '\u{FF1A}',
    '\u{FF20}',
    '\u{FF3B}',
    '\u{2215}',
    '\u{2044}',
    '\u{2100}',
    '\u{2101}',
    '\u{FE57}',
    '\u{FE5F}',
    '\u{FF03}',
    '\u{FF1F}',
    'β',
    'ό',
    'λ',
    'Ж',
    'я',
    'א',
    'ب',
    '\u{0660}',
    '\u{06F0}',
    '中',
    '文',
    '日',
    '本',
    '한',
    'ก',
    '\u{0E33}',
    'ᄀ',
    '\u{1F600}',
    '\u{1F1FA}',
    '\u{10FFFF}',
    '\u{FFFD}',
    '\u{FEFF}',
    '\u{FFFE}',
    '\u{E000}',
    '\u{2028}',
    '\u{00A0}',
    '\u{3000}',
    '\u{0080}',
    '\u{009F}',
    'ÿ',
    '\u{0100}',
];

/// A character for free text in a component: weighted toward what the
/// component's percent-encode set and the parser's state machine look at.
fn text_char(g: &mut Source, out: &mut String) {
    match g.int(0u32..100) {
        0..=34 => out.push(*g.pick(&[
            'a', 'b', 'x', 'z', 'A', 'Z', '0', '1', '9', 'f', 'F', 'e', 'E', 'n', 's', 'X', 'g',
            'G', '-', '_', '.', '~',
        ])),
        35..=59 => out.push(*g.pick(&[
            ' ', '!', '"', '#', '$', '%', '&', '\'', '(', ')', '*', '+', ',', '/', ':', ';', '<',
            '=', '>', '?', '@', '[', '\\', ']', '^', '`', '{', '|', '}', '.', '.',
        ])),
        60..=74 => {
            // A percent escape: valid, of a character that matters, truncated
            // or with non-hex digits.
            out.push_str(&pick_str(
                g,
                &[
                    "%2e",
                    "%2E",
                    "%2f",
                    "%2F",
                    "%5c",
                    "%5C",
                    "%25",
                    "%20",
                    "%00",
                    "%41",
                    "%7e",
                    "%3a",
                    "%3A",
                    "%40",
                    "%23",
                    "%3f",
                    "%5b",
                    "%5d",
                    "%c3%a9",
                    "%C3%A9",
                    "%e2%80%8b",
                    "%ef%bc%8e",
                    "%ff",
                    "%80",
                    "%c0%ae",
                    "%f0%9f%98",
                    "%",
                    "%%",
                    "%2",
                    "%g0",
                    "%0g",
                    "%zz",
                    "%-1",
                ],
            ));
        },
        75..=79 => out.push(char::from(*g.pick(&[0u8, 1, 0x1f, 0x7f, 0x0b, 0x0c]))),
        80..=84 => out.push(*g.pick(&['\t', '\n', '\r'])),
        _ => out.push(*g.pick(UNICODE)),
    }
}

pub(crate) fn text(g: &mut Source, max: usize) -> String {
    if g.bool_with(0.1) {
        return run(g);
    }
    let mut out = String::new();
    let n = few(g, max) + usize::from(g.bool());
    for _ in 0..n {
        text_char(g, &mut out);
    }
    out
}

/// A long run of plain characters with a few text characters at random
/// places: the SIMD scanners of main work on 8-, 16- and 32-byte blocks and
/// hand the rest to scalar code, so lengths around block multiples matter,
/// and so does where in a block the interesting character falls.
fn run(g: &mut Source) -> String {
    let len = match g.int(0u32..3) {
        0 => *g.pick(&[7usize, 8, 9, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65]),
        1 => g.int(1usize..=128),
        _ => g.int(64usize..=600),
    };
    let fill = *g.pick(&['a', 'x', '0', '-', '.', '~', '_']);
    let mut chars: Vec<String> = vec![fill.to_string(); len];
    for _ in 0..few(g, 3) {
        let mut c = String::new();
        text_char(g, &mut c);
        let at = g.index(chars.len());
        chars[at] = c;
    }
    chars.concat()
}

/// Starters for normalization: ASCII and Latin letters, precomposed
/// letters (some decomposing to two or three code points), Hangul jamo
/// (leading, vowel, trailing) and syllables (LV and LVT), singletons that
/// normalize to another code point (`Å` angstrom, `Ω` ohm), composition
/// exclusions, and starters of other scripts.
const STARTERS: &[char] = &[
    'a', 'e', 'o', 'u', 'A', 'z', 'é', 'ǖ', 'ṩ', 'ệ', 'ở', 'Å', 'ä', 'ı', '\u{1100}', '\u{1161}',
    '\u{11A8}', '\u{AC00}', '\u{AC01}', '\u{D7A3}', '\u{212B}', '\u{2126}', '\u{0958}', '\u{FB1D}',
    '\u{0F43}', 'ω', 'ά', 'क', 'ก', 'カ', 'か', '\u{05D0}', '\u{0627}',
];

/// Combining marks of many canonical combining classes (in parentheses):
/// acute and diaeresis (230), grave below and dot below (220), cedilla
/// (202), horn (216), ypogegrammeni (240), Hebrew points (10..), Thai
/// (103), kana voicing (8), nukta (7), Tibetan (129, 130), and U+0344 and
/// U+0F73, which decompose to two marks.
const MARKS: &[char] = &[
    '\u{0301}', '\u{0308}', '\u{0316}', '\u{0323}', '\u{0327}', '\u{031B}', '\u{0345}', '\u{05B0}',
    '\u{05BC}', '\u{0E38}', '\u{3099}', '\u{309A}', '\u{093C}', '\u{0F71}', '\u{0F72}', '\u{0344}',
    '\u{0F73}', '\u{0300}', '\u{0302}', '\u{0306}', '\u{030A}', '\u{0328}', '\u{1DCE}', '\u{20D2}',
];

/// A label that exercises normalization: starters, each followed by a few
/// marks in any order, so that canonical reordering, composition and the
/// NFC quick check all have work to do.
fn nfc_label(g: &mut Source) -> String {
    let mut s = String::new();
    if g.bool_with(0.15) {
        return hangul(g);
    }
    for _ in 0..1 + few(g, 3) {
        s.push(*g.pick(STARTERS));
        for _ in 0..few(g, 4) {
            s.push(*g.pick(MARKS));
        }
    }
    s
}

/// Hangul, which normalization composes arithmetically rather than from
/// the tables: leading, vowel and trailing jamo in and out of order,
/// precomposed LV syllables (U+AC00, U+B098) followed by a trailing jamo
/// (composing to LVT) or by another vowel (not composing), LVT syllables
/// (U+AC01) followed by a trailing jamo, and the boundary jamo U+11A7 and
/// U+11C3, just outside and inside the trailing range.
fn hangul(g: &mut Source) -> String {
    const L: &[char] = &['\u{1100}', '\u{1112}', '\u{1113}'];
    const V: &[char] = &['\u{1161}', '\u{1175}', '\u{1176}'];
    const T: &[char] = &['\u{11A8}', '\u{11C2}', '\u{11A7}', '\u{11C3}'];
    const LV: &[char] = &['\u{AC00}', '\u{B098}', '\u{D788}'];
    const LVT: &[char] = &['\u{AC01}', '\u{D7A3}'];
    let mut s = String::new();
    for _ in 0..1 + few(g, 2) {
        let seq: &[&[char]] = match g.int(0u32..6) {
            0 => &[L, V],
            1 => &[L, V, T],
            2 => &[LV, T],
            3 => &[LV, V],
            4 => &[LVT, T],
            _ => &[T, L, V],
        };
        for set in seq {
            s.push(*g.pick(set));
        }
    }
    s
}

/// An ACE label (`xn--` and punycode): the encoding of a generated label,
/// valid or not once decoded (not normalized, mapped by IDNA, all ASCII,
/// RTL, itself starting with `xn--`, which the standard rejects since
/// whatwg/url#803), or a random tail that may not decode at all (leading,
/// trailing or doubled delimiters; long runs of high digits, which push
/// the decoder's `i`, `w` and `n` past their 32-bit overflow checks; a
/// non-ASCII character among the basic code points).
fn ace_label(g: &mut Source) -> String {
    let prefix = *g.pick(&["xn--", "xn--", "xn--", "XN--", "Xn--"]);
    if g.bool_with(0.6) {
        let source: String = match g.int(0u32..6) {
            5 => format!(
                "{}{}",
                g.pick(&["xn--", "XN--", "xn-", "xn--a-"]),
                (0..1 + few(g, 3))
                    .map(|_| *g.pick(&['é', 'a', '中', '-', 'ß']))
                    .collect::<String>()
            ),
            0 => nfc_label(g),
            1 => (0..1 + few(g, 4)).map(|_| *g.pick(UNICODE)).collect(),
            2 => (0..1 + few(g, 4))
                .map(|_| *g.pick(&['a', 'B', '-', '0', 'é', 'ß', 'ς', '中', 'ب', '\u{337F}']))
                .collect(),
            3 => (0..1 + few(g, 6)).map(|_| *g.pick(&['a', 'b', '-', '1'])).collect(),
            _ => (0..1 + few(g, 3)).map(|_| *g.pick(EXPANDING)).collect(),
        };
        if let Some(code) = idna::punycode::encode_str(&source) {
            let code = if g.bool_with(0.1) { code.to_ascii_uppercase() } else { code };
            return format!("{prefix}{code}");
        }
    }
    let mut tail: String = match g.int(0u32..4) {
        0 => {
            let n = g.int(6usize..=24);
            let run: String = (0..n).map(|_| *g.pick(&['9', '8', '7', 'z', 'y', '0'])).collect();
            if g.bool() {
                format!("{}-{run}", pick_str(g, &["a", "ab", "-", "abc"]))
            } else {
                run
            }
        },
        _ => (0..1 + few(g, 12))
            .map(|_| *g.pick(&['a', 'b', 'k', 'z', '0', '9', '-', 'A', 'Z', 'x', 'n', '8']))
            .collect(),
    };
    if g.bool_with(0.1) {
        let at = g.int(0..=tail.len());
        tail.insert(at, *g.pick(UNICODE));
    }
    format!("{prefix}{tail}")
}

/// A length that grows with size, 1 at the least.
pub(crate) fn sized(g: &mut Source, per: usize) -> usize {
    1 + g.size() / per
}

const SPECIAL: &[&str] = &["http", "https", "ws", "wss", "ftp", "file"];

pub(crate) fn scheme(g: &mut Source) -> String {
    let s = match g.int(0u32..20) {
        0..=11 => pick_str(g, SPECIAL),
        12..=17 => pick_str(
            g,
            &[
                "foo",
                "a",
                "web+demo",
                "a+b-c.d",
                "javascript",
                "data",
                "blob",
                "mailto",
                "urn",
                "sc",
                "git+ssh",
                "x0",
            ],
        ),
        // Not schemes: the parser falls back to a relative URL or fails.
        _ => pick_str(g, &["1a", "+a", "h ttp", "ht\ttp", "", "a_b", "é", "-x"]),
    };
    recase(g, &s)
}

/// What separates the scheme from the authority or path, including the
/// backslashes special schemes accept and too few or too many slashes.
fn slashes(g: &mut Source) -> String {
    weighted(
        g,
        &[
            (12, "//"),
            (2, ""),
            (2, "/"),
            (2, "///"),
            (1, "////"),
            (2, "\\\\"),
            (1, "/\\"),
            (1, "\\/"),
            (1, "\\"),
            (1, "/.//"),
        ],
    )
    .to_string()
}

fn label(g: &mut Source) -> String {
    match g.int(0u32..24) {
        20..=21 => nfc_label(g),
        22..=23 => ace_label(g),
        // Expanding code points, alone.
        8 => (0..1 + few(g, 3)).map(|_| *g.pick(EXPANDING)).collect(),
        0..=7 => {
            let mut s: String = (0..1 + few(g, 8))
                .map(|_| {
                    *g.pick(&['a', 'b', 'e', 'x', 'z', 'q', '0', '1', '9', '-', 'A', 'Z', 'n'])
                })
                .collect();
            if g.bool_with(0.2) {
                s = recase(g, &s);
            }
            s
        },
        9..=10 => pick_str(
            g,
            &[
                "xn--nxasmq6b",
                "xn--",
                "xn--a",
                "xn--zca",
                "xn--ls8h",
                "xn--55qx5d",
                "xn--bcher-kva",
                "xn--1ug",
                "XN--A-ECP",
                "xn--a.",
                "xn--abc-",
                "xn---",
                "xn--ab--",
                "xn--8ca",
                "xn--iun",
                "xN--ZCA",
                "xn--zz",
                "xn--0",
            ],
        ),
        11..=12 => pick_str(
            g,
            &[
                "localhost",
                "LOCALHOST",
                "example",
                "com",
                "test",
                "www",
                "a-b",
                "-a",
                "a-",
                "ab--c",
            ],
        ),
        // Labels that look like numbers make the host an IPv4 candidate.
        13..=14 => pick_str(
            g,
            &["1", "0x1", "0", "08", "09", "0x", "0X", "255", "256", "4294967295", "1e1", "0xg"],
        ),
        _ => {
            let mut s = String::new();
            for _ in 0..1 + few(g, 4) {
                match g.int(0u32..3) {
                    0 => s.push(*g.pick(UNICODE)),
                    1 => s.push(*g.pick(&['a', 'b', 'é', 'ü', 'ß', 'ς', 'ı'])),
                    _ => text_char(g, &mut s),
                }
            }
            s
        },
    }
}

fn domain(g: &mut Source) -> String {
    let max = sized(g, 30).min(5);
    let n = 1 + few(g, max);
    let mut out = if g.bool_with(0.1) {
        // One label many times: what a label costs adds up per label.
        let l = label(g);
        let max = sized(g, 2).min(120);
        let times = 1 + g.int(0..=max);
        vec![l; times].join(".")
    } else {
        (0..n).map(|_| label(g)).collect::<Vec<_>>().join(".")
    };
    if g.bool_with(0.15) {
        out.push_str(&pick_str(g, &[".", "\u{3002}", "..", "\u{FF0E}"]));
    }
    out
}

fn ipv4_part(g: &mut Source) -> String {
    let n: u64 = match g.int(0u32..6) {
        0 => g.int(0u64..=255),
        1 => *g.pick(&[
            0u64, 1, 127, 255, 256, 65535, 65536, 16777215, 16777216, 4294967295, 4294967296,
        ]),
        2 => g.int(0u64..=0xffff_ffff),
        3 => g.int(0u64..=u64::MAX),
        _ => g.int(0u64..=1000),
    };
    match g.int(0u32..10) {
        0..=4 => n.to_string(),
        5..=6 => format!("{}{:x}", if g.bool() { "0x" } else { "0X" }, n),
        7 => format!("0{n:o}"),
        8 => format!("{}{:X}", "0x", n),
        _ => {
            pick_str(g, &["", "0x", "00", "0x0", "09", "0xg", "1a", "0.", "000000000000000000001"])
        },
    }
}

fn ipv4(g: &mut Source) -> String {
    let n = 1 + g.int(0usize..5);
    let mut out = (0..n).map(|_| ipv4_part(g)).collect::<Vec<_>>().join(".");
    if g.bool_with(0.15) {
        out.push('.');
    }
    out
}

fn ipv6(g: &mut Source) -> String {
    let group = |g: &mut Source| -> String {
        match g.int(0u32..8) {
            0..=4 => format!("{:x}", g.int(0u32..=0xffff)),
            5 => format!("{:04X}", g.int(0u32..=0xffff)),
            6 => pick_str(g, &["0", "0000", "ffff", "FFFF", "00000", "12345", "g", ""]),
            _ => format!("{:x}", g.int(0u32..=0xf)),
        }
    };
    let n = g.int(0usize..=9);
    let mut groups: Vec<String> = (0..n).map(|_| group(g)).collect();
    if g.bool_with(0.6) {
        let at = g.int(0..=groups.len());
        groups.insert(at, String::new());
        if at == 0 || at == groups.len() - 1 {
            groups.insert(at, String::new());
        }
    }
    let mut out = groups.join(":");
    if g.bool_with(0.25) {
        if !out.is_empty() && !out.ends_with(':') {
            out.push(':');
        }
        out.push_str(&ipv4(g));
    }
    if g.bool_with(0.05) {
        out.push_str(&pick_str(g, &["%25eth0", "%eth0", "%", "]", ":::", "::"]));
    }
    format!("[{out}]")
}

fn opaque_host(g: &mut Source) -> String {
    let mut s = String::new();
    for _ in 0..1 + few(g, 6) {
        text_char(g, &mut s);
    }
    s.retain(|c| !matches!(c, '/' | '?' | '#' | '\\'));
    s
}

pub(crate) fn host(g: &mut Source) -> String {
    match g.int(0u32..20) {
        0..=8 => domain(g),
        9..=12 => ipv4(g),
        13..=15 => ipv6(g),
        16..=17 => opaque_host(g),
        18 => String::new(),
        // A host written with percent escapes, which decode before the
        // host is parsed.
        _ => pick_str(
            g,
            &[
                "%31%32%37.0.0.1",
                "ex%61mple.com",
                "%5B::1%5D",
                "%2e",
                "a%2Eb",
                "%zz",
                "xn%2D%2Dzca",
                "%ef%bc%8e",
            ],
        ),
    }
}

pub(crate) fn port(g: &mut Source) -> String {
    match g.int(0u32..10) {
        0..=3 => pick_str(
            g,
            &[
                "80",
                "443",
                "21",
                "0",
                "65535",
                "65536",
                "8080",
                "1",
                "00080",
                "000000000000000000080",
            ],
        ),
        4..=5 => g.int(0u64..=99999).to_string(),
        6 => g.int(0u64..=u64::MAX).to_string(),
        7 => String::new(),
        _ => pick_str(
            g,
            &[
                "8a", "a", "-1", "+1", " 80", "80 ", "8\t0", "0x50", "１", "80/", "80?", "80#",
                "80\\",
            ],
        ),
    }
}

fn authority(g: &mut Source) -> String {
    let mut out = String::new();
    if g.bool_with(0.2) {
        out.push_str(&text(g, 4));
        if g.bool() {
            out.push(':');
            out.push_str(&text(g, 4));
        }
        out.push('@');
        if g.bool_with(0.1) {
            out.push_str(&pick_str(g, &["@", "u@", ":@", "@@"]));
        }
    }
    out.push_str(&host(g));
    if g.bool_with(0.25) {
        out.push(':');
        out.push_str(&port(g));
    }
    out
}

fn segment(g: &mut Source) -> String {
    match g.int(0u32..20) {
        0..=3 => pick_str(
            g,
            &[
                ".",
                "..",
                "%2e",
                "%2E",
                ".%2e",
                "%2e.",
                "%2e%2E",
                "%2E%2e",
                ".%2E",
                "...",
                "%2e%2e%2e",
                "..%00",
            ],
        ),
        4 => pick_str(g, &["C:", "c|", "C|", "a:", "z:", "C:%2f", "C%3A", "C%7C", "CC:", "1:"]),
        5..=12 => {
            let mut s = String::new();
            for _ in 0..1 + few(g, 6) {
                s.push(*g.pick(&['a', 'b', 'x', 'Z', '0', '-', '_', '~', '.']));
            }
            s
        },
        _ => {
            let mut s = text(g, 6);
            s.retain(|c| !matches!(c, '?' | '#'));
            s
        },
    }
}

pub(crate) fn path(g: &mut Source) -> String {
    let max = sized(g, 10).min(8);
    let n = few(g, max);
    let mut out = String::new();
    for _ in 0..n {
        out.push_str(weighted(g, &[(8, "/"), (2, "\\"), (1, "//")]));
        out.push_str(&segment(g));
    }
    if n > 0 && g.bool_with(0.15) {
        out.push('/');
    }
    out
}

pub(crate) fn query(g: &mut Source) -> String {
    let max = sized(g, 8).min(10);
    let mut s = text(g, max);
    s.retain(|c| c != '#');
    s
}

/// The noise the parser strips first: leading and trailing C0 controls and
/// spaces, and tabs and newlines anywhere.
fn noise(g: &mut Source, s: String) -> String {
    let mut s = s;
    if g.bool_with(0.1) {
        let mut chars: Vec<char> = s.chars().collect();
        for _ in 0..1 + few(g, 3) {
            let at = g.int(0..=chars.len());
            chars.insert(at, *g.pick(&['\t', '\n', '\r']));
        }
        s = chars.into_iter().collect();
    }
    if g.bool_with(0.08) {
        let lead: String =
            (0..1 + few(g, 2)).map(|_| *g.pick(&[' ', '\u{0}', '\u{1f}', '\t', '\u{c}'])).collect();
        s.insert_str(0, &lead);
    }
    if g.bool_with(0.08) {
        for _ in 0..1 + few(g, 2) {
            s.push(*g.pick(&[' ', '\u{0}', '\u{1f}', '\n', '\u{b}']));
        }
    }
    s
}

/// An absolute URL.
pub fn absolute(g: &mut Source) -> String {
    // A canonical, noise-free absolute URL: the shape ada's fast paths take
    // (try_can_parse_clean_http, finish_simple_absolute_with_port). No
    // backslashes, tabs, or odd characters, so the scanners run to the end.
    if g.bool_with(0.15) {
        let s = *g.pick(&["http", "https", "ws", "wss", "ftp"]);
        let host: String = (0..1 + few(g, 2))
            .map(|_| {
                (0..1 + few(g, 6))
                    .map(|_| *g.pick(&['a', 'b', 'x', 'z', '0', '9', '-']))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(".");
        let port =
            if g.bool_with(0.4) { format!(":{}", g.int(0u32..=65535)) } else { String::new() };
        let segs = few(g, 4);
        let path: String = (0..segs)
            .map(|_| {
                format!(
                    "/{}",
                    (0..few(g, 6))
                        .map(|_| *g.pick(&['a', 'b', 'x', '0', '-', '_', '.']))
                        .collect::<String>()
                )
            })
            .collect();
        let mut out = format!("{s}://{host}{port}{path}");
        if g.bool_with(0.3) {
            out.push_str(&format!(
                "?{}",
                (0..1 + few(g, 6)).map(|_| *g.pick(&['a', 'b', '=', '&', '0'])).collect::<String>()
            ));
        }
        if g.bool_with(0.2) {
            out.push_str(&format!(
                "#{}",
                (0..few(g, 5)).map(|_| *g.pick(&['a', 'b', '0'])).collect::<String>()
            ));
        }
        return out;
    }
    let mut out = scheme(g);
    out.push(':');
    let opaque = g.bool_with(0.1);
    if opaque {
        // An opaque path: no slash after the scheme. A `blob:` URL's origin
        // is that of the URL its path parses as, when that is `http(s):`,
        // so its path is mostly a whole URL (of a special scheme or not).
        if out.eq_ignore_ascii_case("blob:") && g.bool_with(0.7) {
            out.push_str(&absolute(g));
        } else {
            out.push_str(&text(g, 6));
        }
    } else {
        let s = slashes(g);
        let has_authority = s.len() >= 2 || g.bool_with(0.1);
        out.push_str(&s);
        if has_authority {
            out.push_str(&authority(g));
        }
        out.push_str(&path(g));
    }
    if g.bool_with(0.3) {
        out.push('?');
        out.push_str(&query(g));
    }
    if g.bool_with(0.25) {
        out.push('#');
        out.push_str(&text(g, 6));
    }
    // Heavy tail: now and then stretch one piece far past the usual lengths,
    // to reach the parser's buffer-growth and error-recovery states
    // (parse_url_impl) that short inputs never do. Rendered as before, so it
    // stays a valid parser input and the differential oracle still applies.
    if g.bool_with(0.06) {
        out = heavy_tail(g, out);
    }
    out
}

/// Stretches one slice of `url` into a long run: a deep path, a long host
/// label, a long percent-escape run, many credentials separators, or a long
/// query or fragment. Kept to characters the component allows so the result
/// is still a URL a parser accepts, not a reject.
fn heavy_tail(g: &mut Source, url: String) -> String {
    let n = *g.pick(&[200usize, 1000, 4096, 10000]);
    let blob: String = match g.int(0u32..6) {
        0 => "/a".repeat(n / 2),
        1 => "a".repeat(n),
        2 => "%41".repeat(n / 3),
        3 => "%c3%a9".repeat(n / 6),
        4 => g.pick(&['.', '/', '\\', '@', ':']).to_string().repeat(n),
        _ => (0..n / 8).map(|_| *g.pick(UNICODE)).collect(),
    };
    // Splice the run at a slash, ?, # or @ boundary when there is one, so it
    // lands inside a component rather than the scheme.
    match url.rfind(['/', '?', '#', '@']) {
        Some(at) => format!("{}{blob}{}", &url[..=at], &url[at + 1..]),
        None => format!("{url}{blob}"),
    }
}

/// The scheme `url` starts with, as written, when it starts with one.
fn scheme_of(url: &str) -> Option<&str> {
    let (s, _) = url.split_once(':')?;
    let mut chars = s.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then_some(s)
}

/// A relative reference, resolved against `base`.
fn relative(g: &mut Source, base: &str) -> String {
    let mut out = match g.int(0u32..10) {
        0 => format!("//{}{}", authority(g), path(g)),
        1 => format!("\\\\{}{}", authority(g), path(g)),
        2..=4 => path(g),
        5..=6 => path(g).trim_start_matches(['/', '\\']).to_owned(),
        // A reference that names a scheme. When it is the base's own special
        // scheme, the parser goes to the special relative or authority state,
        // where what follows the colon decides: `//` or `\\` starts an
        // authority, anything else is relative to the base's path. A
        // different scheme makes the reference absolute despite the base
        // (and the parser's relative fast path must turn it down). So the
        // scheme is mostly the base's, recased, and the colon is followed by
        // every kind of slash run, with or without an authority.
        7 => {
            let s = match scheme_of(base) {
                Some(s) if g.bool_with(0.7) => s.to_owned(),
                _ => pick_str(g, SPECIAL),
            };
            let s = recase(g, &s);
            let rest = match g.int(0u32..3) {
                0 => path(g).trim_start_matches(['/', '\\']).to_owned(),
                1 => path(g),
                _ => format!("{}{}{}", slashes(g), authority(g), path(g)),
            };
            format!("{s}:{rest}")
        },
        8 => pick_str(
            g,
            &[
                "", ".", "..", "./", "../..", "/..", "C:", "/C:", "c|/x", "//", "///", "?", "#",
                "\\",
            ],
        ),
        _ => String::new(),
    };
    if g.bool_with(0.3) {
        out.push('?');
        out.push_str(&query(g));
    }
    if g.bool_with(0.25) {
        out.push('#');
        out.push_str(&text(g, 6));
    }
    out
}

/// A Windows drive letter as written: normalized (`C:`) or not (`C|`), any
/// case, and near misses (two letters, a digit, an escaped colon).
fn drive(g: &mut Source) -> String {
    weighted(g, &[(4, "C:"), (2, "c:"), (3, "C|"), (1, "z|"), (1, "CC:"), (1, "1:"), (1, "C%3A")])
        .to_string()
}

/// A `file:` base for [`file_reference`]: an empty, `localhost` or named
/// host, and a path whose first segment is a drive letter most of the time.
fn file_base(g: &mut Source) -> String {
    let host = *g.pick(&["", "", "localhost", "h", "LOCALHOST", "[::1]"]);
    let mut out = format!("file://{host}");
    if g.bool_with(0.75) {
        out.push('/');
        out.push_str(&drive(g));
    }
    out.push_str(&path(g));
    if g.bool_with(0.2) {
        out.push('?');
        out.push_str(&query(g));
    }
    out
}

/// A reference against a `file:` base. The file, file slash and file host
/// states test the reference's leading drive letter and the base's first
/// path segment together (whether to keep the base's host, inherit its drive
/// letter, shorten its path or clear it), so independent draws of base and
/// reference almost never pair them: these do, with every number of leading
/// slashes, a drive letter or not, and a host that is itself a drive letter.
fn file_reference(g: &mut Source) -> String {
    let lead =
        *g.pick(&["", "", "/", "\\", "//", "\\\\", "///", "//h/", "file:", "file:/", "FILE://"]);
    let mut out = lead.to_owned();
    match g.int(0u32..4) {
        0 => {},
        1 => out.push_str(&drive(g)),
        _ => {
            out.push_str(&drive(g));
            out.push_str(weighted(g, &[(4, "/"), (2, "\\"), (1, ""), (1, "?"), (1, "#")]));
        },
    }
    out.push_str(path(g).trim_start_matches(['/', '\\']));
    if g.bool_with(0.2) {
        out.push_str(&pick_str(g, &["..", "/..", "/.", "%2e%2e"]));
    }
    if g.bool_with(0.2) {
        out.push('?');
        out.push_str(&query(g));
    }
    if g.bool_with(0.15) {
        out.push('#');
        out.push_str(&text(g, 4));
    }
    out
}

/// Values the host and hostname setters treat on their own, apart from host
/// parsing: the empty string (clears a non-special host; fails on a special
/// URL and on one with credentials or a port), `localhost` (cleared on
/// `file:`, also when percent-encoded), an empty host before a port (the
/// hostname setter fails on any colon), an empty port, and a host cut short
/// by `/`, `?`, `#` or `\`. The grammar's hosts reach these rarely, and only
/// once noise and suffixes have been added.
const HOST_EDGES: &[&str] = &[
    "",
    "",
    "localhost",
    "LocalHost",
    "localhost:80",
    "%6c%6f%63%61%6c%68%6f%73%74",
    ":",
    ":80",
    "h:",
    "h:80",
    "[::1]:",
    "[::1]:8",
    "h/x",
    "h?q",
    "h#f",
    "h\\x",
    "/",
    "?",
    "#",
    "\\",
    "h:/",
];

/// A value for `setter`, drawn from the grammar of what it sets, with
/// whatever else may follow it in a URL now and then.
fn value(g: &mut Source, setter: Setter) -> String {
    let mut v = match setter {
        Setter::Href => absolute(g),
        Setter::Protocol => {
            let mut s = scheme(g);
            if g.bool() {
                s.push(':');
            }
            s
        },
        Setter::Username | Setter::Password => text(g, 6),
        Setter::Host | Setter::Hostname if g.bool_with(0.25) => pick_str(g, HOST_EDGES),
        Setter::Host => {
            let mut s = host(g);
            if g.bool_with(0.3) {
                s.push(':');
                s.push_str(&port(g));
            }
            s
        },
        Setter::Hostname => host(g),
        Setter::Port => port(g),
        Setter::Pathname => {
            let p = path(g);
            if g.bool_with(0.3) {
                p.trim_start_matches('/').to_owned()
            } else {
                p
            }
        },
        Setter::Search => {
            let q = query(g);
            if g.bool() {
                format!("?{q}")
            } else {
                q
            }
        },
        Setter::Hash => {
            let h = text(g, 6);
            if g.bool() {
                format!("#{h}")
            } else {
                h
            }
        },
    };
    if g.bool_with(0.1) {
        v.push_str(&pick_str(g, &["/x", "?q", "#f", ":1", "@h", "\\", "/", ":", " ", "%"]));
    }
    noise(g, v)
}

fn setter(g: &mut Source) -> Setter {
    *g.pick(&Setter::ALL)
}

fn case(g: &mut Source) -> Case {
    let (input, base) = match g.int(0u32..20) {
        0..=13 => (absolute(g), None),
        14 => {
            let base = file_base(g);
            (file_reference(g), Some(base))
        },
        _ => {
            let base = absolute(g);
            (relative(g, &base), Some(base))
        },
    };
    let mut case = Case { input: noise(g, input), base, sets: Vec::new() };
    if g.bool_with(0.5) {
        let max = sized(g, 20).min(6);
        for _ in 0..1 + few(g, max) {
            let s = setter(g);
            case.sets.push((s, value(g, s)));
        }
    }
    case
}

/// Strings shorter than `s`: a run of characters deleted, halves first,
/// down to single characters.
pub(crate) fn deletions(
    s: &str,
    visit: &mut dyn FnMut(String) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let chars: Vec<char> = s.chars().collect();
    let mut run = chars.len();
    while run > 0 {
        let mut at = 0;
        while at < chars.len() {
            let end = (at + run).min(chars.len());
            visit(chars[..at].iter().chain(&chars[end..]).collect())?;
            at += run;
        }
        run /= 2;
    }
    ControlFlow::Continue(())
}

/// `s` with one character other than `a` replaced by `a`.
pub(crate) fn simplifications(
    s: &str,
    visit: &mut dyn FnMut(String) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c != 'a' {
            let mut out = chars.clone();
            out[i] = 'a';
            visit(out.into_iter().collect())?;
        }
    }
    ControlFlow::Continue(())
}

/// The strings of a case, by position: the input, the base, the values.
fn with_string(case: &Case, at: usize, s: String) -> Case {
    let mut out = case.clone();
    match at {
        0 => out.input = s,
        1 => out.base = Some(s),
        k => out.sets[k - 2].1 = s,
    }
    out
}

fn strings(case: &Case) -> Vec<(usize, &str)> {
    let mut out = vec![(0, case.input.as_str())];
    if let Some(b) = &case.base {
        out.push((1, b.as_str()));
    }
    out.extend(case.sets.iter().enumerate().map(|(k, (_, v))| (k + 2, v.as_str())));
    out
}

/// A run of `s`'s characters replaced by `fragment`.
fn splice(g: &mut Source, s: &str, fragment: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let at = g.int(0..=chars.len());
    let end = (at + few(g, 8)).min(chars.len());
    chars[..at]
        .iter()
        .copied()
        .chain(fragment.chars())
        .chain(chars[end..].iter().copied())
        .collect()
}

/// A fragment of the grammar, to splice into a string.
fn fragment(g: &mut Source) -> String {
    match g.int(0u32..10) {
        0 => host(g),
        1 => format!(":{}", port(g)),
        2 => path(g),
        3 => segment(g),
        4 => text(g, 4),
        5 => format!("{}:", scheme(g)),
        6 => slashes(g),
        7 => authority(g),
        8 => format!("?{}", query(g)),
        _ => format!("#{}", text(g, 4)),
    }
}

/// Cases: see the module documentation.
pub struct Cases;

impl Gen for Cases {
    type Value = Case;

    fn generate(&self, g: &mut Source) -> Case {
        case(g)
    }

    fn shrink(
        &self,
        case: &Case,
        visit: &mut dyn FnMut(Case) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for k in (0..case.sets.len()).rev() {
            let mut out = case.clone();
            out.sets.remove(k);
            visit(out)?;
        }
        if case.base.is_some() {
            visit(Case { base: None, ..case.clone() })?;
            // The base resolved against nothing, standing in for the input.
            visit(Case { input: case.base.clone().unwrap(), base: None, sets: case.sets.clone() })?;
        }
        for (at, s) in strings(case) {
            deletions(s, &mut |t| visit(with_string(case, at, t)))?;
        }
        for (at, s) in strings(case) {
            simplifications(s, &mut |t| visit(with_string(case, at, t)))?;
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, case: &Case, g: &mut Source) -> Case {
        match g.int(0u32..10) {
            0..=4 => {
                let all = strings(case);
                let (at, s) = all[g.index(all.len())];
                let f = fragment(g);
                with_string(case, at, splice(g, s, &f))
            },
            5..=6 => {
                let mut out = case.clone();
                let s = setter(g);
                let v = value(g, s);
                if out.sets.is_empty() || g.bool() {
                    let at = g.int(0..=out.sets.len());
                    out.sets.insert(at, (s, v));
                } else {
                    let at = g.index(out.sets.len());
                    out.sets[at] = (s, v);
                }
                out
            },
            7 => {
                let all = strings(case);
                let (at, s) = all[g.index(all.len())];
                let s = s.to_owned();
                with_string(case, at, noise(g, s))
            },
            8 => match &case.base {
                Some(_) => Case { base: Some(absolute(g)), ..case.clone() },
                None => Case {
                    input: relative(g, &case.input),
                    base: Some(case.input.clone()),
                    sets: case.sets.clone(),
                },
            },
            _ => self.generate(g),
        }
    }
}

fn has_noise(s: &str) -> bool {
    s.contains(['\t', '\n', '\r'])
        || s.starts_with(|c: char| c <= ' ')
        || s.ends_with(|c: char| c <= ' ')
}

/// Numbers about a case: its strings' lengths, and the shapes the grammar
/// aims at, as indicators of the input (0 or 1) unless said otherwise.
pub fn case_metrics(case: &Case, _size: usize) -> Vec<(&'static str, f64)> {
    let input = case.input.as_str();
    let lower = input.to_ascii_lowercase();
    let b = |x: bool| f64::from(u8::from(x));
    let total: usize = strings(case).iter().map(|(_, s)| s.chars().count()).sum();
    vec![
        // Characters in the input, the base and every setter value.
        ("chars", total as f64),
        ("setters", case.sets.len() as f64),
        ("relative", b(case.base.is_some())),
        (
            "special scheme",
            b(SPECIAL.iter().any(|s| lower.trim_start().starts_with(&format!("{s}:")))),
        ),
        ("non-ASCII", b(!input.is_ascii())),
        ("percent escapes", input.matches('%').count() as f64),
        ("IPv6", b(input.contains('['))),
        ("credentials", b(input.contains('@'))),
        ("dot segments", b(input.contains("/.") || lower.contains("%2e"))),
        ("noise", b(has_noise(input))),
        ("backslash", b(input.contains('\\'))),
    ]
}

/// The generator of cases, for properties.
#[ephyra::generator(metrics = case_metrics)]
pub fn cases() -> impl Gen<Value = Case> {
    Cases
}
