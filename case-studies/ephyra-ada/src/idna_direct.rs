//! NOT A SOUND ORACLE, kept only for exploration. ada's `to_ascii` and
//! `to_unicode` implement the URL Standard's domain-to-ASCII/Unicode (an
//! ASCII fast path that passes `xn--` labels through without re-validating,
//! and no IDNA re-mapping in `to_unicode`), while tr46's `toASCII`/`toUnicode`
//! are pure UTS-46. They differ by design on ASCII ACE labels and on mapping,
//! so a direct differential floods false differences (about 63% here). Every
//! difference checked leaked through to neither URL API: ada and whatwg-url
//! agree on the same inputs as hosts (e.g. `http://xn--a.b/`). The sound IDNA
//! oracle is host parsing against whatwg-url (`ada_agrees_with_reference`),
//! where the stale-tables bug was found. `examples/idna_triage.rs` buckets
//! the differences for anyone who wants to look.
//!
//! ada's IDNA called directly (`ada::idna::to_ascii` / `to_unicode`), on a
//! much broader distribution than hostnames allow: many labels, raw
//! punycode, bidi and joiner contexts, full-width and compatibility forms,
//! every kind of dot. The reference is tr46 in Node (whatwg-url's IDNA), run
//! with the flags ada uses: CheckHyphens off, CheckBidi and CheckJoiners on,
//! UseSTD3 off, non-transitional, no DNS-length check. tr46 is Unicode 17,
//! matching the patched ada vendored here.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};
use serde_json::json;

use crate::{gen, params::text};

/// What ada's IDNA makes of a domain: whether `to_ascii` succeeded and its
/// output, and `to_unicode`'s output (best effort, never fails).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdnaResult {
    pub to_ascii_ok: bool,
    pub to_ascii: String,
    pub to_unicode: String,
}

extern "C" {
    fn ea_idna(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_free(p: *mut u8);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// ada's results for each domain.
pub fn ada(domains: &[String]) -> Vec<IdnaResult> {
    let mut bytes = (domains.len() as u32).to_le_bytes().to_vec();
    for d in domains {
        put_str(&mut bytes, d);
    }
    let mut out = std::ptr::null_mut();
    // SAFETY: `bytes` is live for the call; the shim writes a malloc'd
    // buffer of the returned length to `out`, copied and then freed.
    let t = crate::ada::locked(|| unsafe {
        let len = ea_idna(bytes.as_ptr(), bytes.len(), &mut out);
        let t = std::slice::from_raw_parts(out, len).to_vec();
        ea_free(out);
        t
    });
    let mut r = &t[..];
    let str = |r: &mut &[u8]| {
        let n = u32::from_le_bytes(r[..4].try_into().unwrap()) as usize;
        let s = text(&r[4..4 + n]);
        *r = &r[4 + n..];
        s
    };
    domains
        .iter()
        .map(|_| {
            let ok = r[0] == 1;
            r = &r[1..];
            let a = str(&mut r);
            let u = str(&mut r);
            IdnaResult { to_ascii_ok: ok, to_ascii: a, to_unicode: u }
        })
        .collect()
}

/// tr46's results for each domain, with ada's flags. `to_ascii_ok` is
/// whether `toASCII` returned non-null; `to_unicode` is the domain `toUnicode`
/// returns (its error flag is ignored, as ada's to_unicode has none).
pub fn reference(domains: &[String]) -> Vec<IdnaResult> {
    let v = crate::reference::request(&json!({ "idna": domains }));
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| IdnaResult {
            to_ascii_ok: r["asciiOk"].as_bool().unwrap(),
            to_ascii: r["ascii"].as_str().unwrap_or("").to_owned(),
            to_unicode: r["unicode"].as_str().unwrap().to_owned(),
        })
        .collect()
}

/// Code points that drive IDNA: mapped forms, the dots that split labels,
/// deviation characters, joiners and a virama, bidi letters and digits,
/// and ASCII that is forbidden or special in a host.
const CPS: &[char] = &[
    'a',
    'B',
    '0',
    '9',
    '-',
    '.',
    'x',
    'é',
    'ß',
    'ς',
    'ı',
    'İ',
    'ﬀ',
    'Ⅻ',
    '™',
    'ª',
    'µ',
    '\u{00AD}',
    '\u{200C}',
    '\u{200D}',
    '\u{0301}',
    '\u{0308}',
    '\u{094D}',
    '\u{3002}',
    '\u{FF0E}',
    '\u{FF61}',
    '\u{2024}',
    'Ａ',
    'ｚ',
    '０',
    '\u{FF05}',
    '\u{FF0F}',
    'β',
    'ό',
    'ا',
    'ب',
    '\u{0660}',
    '\u{06F0}',
    '中',
    '한',
    '\u{10940}',
    '\u{08CB}',
    '\u{1F600}',
    '\u{FFFD}',
    '\u{FEFF}',
    '\u{E000}',
    '_',
    '~',
    ' ',
    '/',
    '%',
];

/// A punycode-looking ACE label built from the `idna` crate's encoder, so
/// the decode path (`punycode_to_utf32`, re-encode checks) is exercised.
fn ace_label(g: &mut Source) -> String {
    let source: String = (0..1 + gen::few(g, 4)).map(|_| *g.pick(CPS)).collect();
    match idna::punycode::encode_str(&source) {
        Some(code) if !code.is_empty() => format!("xn--{code}"),
        _ => "xn--a".to_owned(),
    }
}

fn label(g: &mut Source) -> String {
    match g.int(0u32..10) {
        0..=4 => (0..1 + gen::few(g, 6)).map(|_| *g.pick(CPS)).collect(),
        5..=6 => ace_label(g),
        7 => gen::pick_str(
            g,
            &[
                "xn--",
                "xn--a",
                "xn--zca",
                "xn--nxasmq6b",
                "xn---",
                "xn--a-",
                "XN--A",
                "xn--ls8h",
                "xn--0",
                "xn--bcher-kva",
                "xn--a.b",
                "xn--tešt",
            ],
        ),
        8 => gen::pick_str(
            g,
            &["a", "example", "localhost", "", "-a-", "a--b", "3", "0x1", " \u{200d}a"],
        ),
        _ => {
            // A long run, to reach punycode overflow and length paths.
            let c = *g.pick(CPS);
            c.to_string().repeat(*g.pick(&[1usize, 30, 63, 64, 200]))
        },
    }
}

/// A domain: labels joined by a separator that IDNA treats as a dot.
fn domain(g: &mut Source) -> String {
    let n = 1 + gen::few(g, 4);
    let dots = [".", ".", "\u{3002}", "\u{FF0E}", "\u{FF61}"];
    let parts: Vec<String> = (0..n).map(|_| label(g)).collect();
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push_str(g.pick(&dots));
        }
        out.push_str(p);
    }
    if g.bool_with(0.15) {
        out.push_str(g.pick(&dots));
    }
    out
}

/// Lists of domains for the IDNA differential. Shrinking drops domains then
/// characters; mutation redraws a domain or a label.
pub struct IdnaCases;

impl Gen for IdnaCases {
    type Value = Vec<String>;

    fn generate(&self, g: &mut Source) -> Vec<String> {
        (0..1 + gen::few(g, 3)).map(|_| domain(g)).collect()
    }

    fn shrink(
        &self,
        v: &Vec<String>,
        visit: &mut dyn FnMut(Vec<String>) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if v.len() > 1 {
            for k in (0..v.len()).rev() {
                let mut out = v.clone();
                out.remove(k);
                visit(out)?;
            }
        }
        for k in 0..v.len() {
            gen::deletions(&v[k], &mut |t| {
                let mut out = v.clone();
                out[k] = t;
                visit(out)
            })?;
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, v: &Vec<String>, g: &mut Source) -> Vec<String> {
        let mut out = v.clone();
        let k = g.index(out.len());
        out[k] = domain(g);
        out
    }
}

/// Numbers about a domain list: count, labels, and non-ASCII share.
#[allow(clippy::ptr_arg)]
pub fn idna_metrics(v: &Vec<String>, _size: usize) -> Vec<(&'static str, f64)> {
    let labels: usize = v.iter().map(|d| d.split('.').count()).sum();
    vec![
        ("domains", v.len() as f64),
        ("labels", labels as f64),
        ("non-ASCII", f64::from(u8::from(v.iter().any(|d| !d.is_ascii())))),
    ]
}

/// The generator of IDNA domain lists.
#[ephyra::generator(metrics = idna_metrics)]
pub fn idna_cases() -> impl Gen<Value = Vec<String>> {
    IdnaCases
}
