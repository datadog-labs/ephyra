// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Canonical URLs as a verified validity condition: [`is_canonical`] holds
//! when ada parses the string and its href equals it. [`canonical_urls`]
//! generates only such strings, and `ephyra check` verifies that its
//! shrinker and mutator keep them canonical. This is the analog of the
//! SQLite work's `integrity_ok`: ada judges what a canonical URL is, and the
//! generator explores the valid region densely, where arbitrary strings
//! mostly fail to parse. A counterexample is a minimal canonical URL.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};

use crate::{
    ada::{self, Kind},
    gen, Case,
};

/// The canonical href ada gives `input`, if it parses (aggregator).
fn canonicalize(input: &str) -> Option<String> {
    let (t, _) = ada::run(Kind::Aggregator, &Case::parse(input.to_owned()), 0);
    t.steps.into_iter().next().filter(|s| s.ok).and_then(|s| s.url).map(|u| u.href)
}

/// A string ada parses to itself: `parse(s).href == s`.
#[ephyra::generator]
#[allow(clippy::ptr_arg)]
pub fn is_canonical(s: &String) -> bool {
    canonicalize(s).as_deref() == Some(s.as_str())
}

/// Canonical URLs. Generation canonicalizes a grammar URL through ada.
/// Shrinking deletes characters and re-canonicalizes, keeping only
/// candidates that still parse and are strictly shorter (so the measure,
/// the href length, falls and chains settle). Mutation canonicalizes a
/// mutated raw string, falling back to the value when that is not canonical.
pub struct Canonical;

impl Gen for Canonical {
    type Value = String;

    fn generate(&self, g: &mut Source) -> String {
        for _ in 0..40 {
            if let Some(h) = canonicalize(&gen::absolute(g)) {
                return h;
            }
        }
        // A scheme-only URL always canonicalizes to itself.
        "a:".to_owned()
    }

    fn shrink(
        &self,
        s: &String,
        visit: &mut dyn FnMut(String) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let mut seen = std::collections::HashSet::new();
        gen::deletions(s, &mut |t| {
            match canonicalize(&t) {
                // Keep canonical candidates strictly shorter than the value,
                // each visited once.
                Some(h) if h.len() < s.len() && seen.insert(h.clone()) => visit(h),
                _ => ControlFlow::Continue(()),
            }
        })
    }

    fn mutate(&self, s: &String, g: &mut Source) -> String {
        // Splice a grammar fragment into the raw href, re-canonicalize; if
        // that does not parse, keep the (canonical) value.
        let chars: Vec<char> = s.chars().collect();
        let at = g.int(0..=chars.len());
        let frag = gen::absolute(g);
        let spliced: String = chars[..at].iter().collect::<String>()
            + &frag
            + &chars[at..].iter().collect::<String>();
        canonicalize(&spliced).unwrap_or_else(|| s.clone())
    }
}

/// Numbers about a canonical URL.
#[allow(clippy::ptr_arg)]
pub fn canonical_metrics(s: &String, _size: usize) -> Vec<(&'static str, f64)> {
    let b = |x: bool| f64::from(u8::from(x));
    vec![
        ("chars", s.chars().count() as f64),
        (
            "special",
            b(["http://", "https://", "ws://", "wss://", "ftp://", "file://"]
                .iter()
                .any(|p| s.starts_with(p))),
        ),
        ("has query", b(s.contains('?'))),
        ("has fragment", b(s.contains('#'))),
        ("non-ASCII", b(!s.is_ascii())),
    ]
}

/// The generator of canonical URLs.
#[ephyra::generator(is_canonical, metrics = canonical_metrics)]
pub fn canonical_urls() -> impl Gen<Value = String> {
    Canonical
}
