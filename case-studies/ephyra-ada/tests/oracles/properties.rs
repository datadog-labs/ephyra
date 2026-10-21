// The properties, shared with ../ada-asan by include!: no inner attributes
// or doc comments here.

use ephyra_ada::{
    ada::{self, Kind},
    gen::cases,
    reference, Case, Parts, Setter, Step, Transcript,
};
use unicode_bidi::{bidi_class, BidiClass};

/// Whether any string of the case, as written or percent-decoded, has a
/// character of an RTL label (bidi class R, AL or AN), as written or in an
/// ACE label. ada applies the
/// RFC 5893 Bidi rule only to labels with such a character of their own,
/// not to every label of a Bidi domain name as UTS 46 says, so it accepts
/// hosts like `ب.-` that whatwg-url rejects. Known: ada-url/ada#1251 (closed
/// in favour of ada-url/idna#91, the library ada vendors its IDNA from).
/// Disagreements on such cases are excused, not reported.
fn may_be_bidi_domain(case: &Case) -> bool {
    let rtl = |s: &str| s.chars().any(|c| matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::AN));
    // RTL characters may also hide in ACE labels: decode every run that
    // looks like one.
    let ace = |s: &str| {
        s.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .filter_map(|l| l.get(..4).filter(|p| p.eq_ignore_ascii_case("xn--")).map(|_| &l[4..]))
            .any(|code| idna::punycode::decode_to_string(&code.to_ascii_lowercase()).is_some_and(|u| rtl(&u)))
    };
    let strings = std::iter::once(&case.input).chain(&case.base).chain(case.sets.iter().map(|(_, v)| v));
    strings.into_iter().any(|s| {
        // The parser drops tabs and newlines first, so they may split an ACE
        // label as written (`xn--n\rgb` is `xn--ngb`).
        let s = &s.replace(['\t', '\n', '\r'], "");
        let decoded = percent_encoding::percent_decode_str(s).decode_utf8_lossy();
        rtl(s) || rtl(&decoded) || ace(s) || ace(&decoded)
    })
}

fn urls(t: &Transcript) -> Vec<Option<Parts>> {
    t.steps.iter().map(|s| s.url.clone()).collect()
}

/// `ada::url` and `ada::url_aggregator` give the same transcript, and the
/// aggregator's offsets are consistent after every step.
#[ephyra::test(gen = cases(), tests = 3000)]
fn representations_agree(case: Case) {
    let (url, _) = ada::run(Kind::Url, &case, 0);
    let (agg, valid) = ada::run(Kind::Aggregator, &case, 0);
    assert!(valid, "url_aggregator::validate() failed");
    assert_eq!(url, agg);
}

/// `can_parse` answers whether the parse succeeds.
#[ephyra::test(gen = cases(), tests = 3000)]
fn can_parse_agrees_with_parse(case: Case) {
    for kind in [Kind::Url, Kind::Aggregator] {
        let (t, _) = ada::run(kind, &case, 0);
        assert_eq!(t.can_parse, t.steps[0].ok, "{kind:?}");
    }
}

/// Every href ada serializes parses back to the same URL, unless it does
/// not in whatwg-url either: the standard itself does not round-trip every
/// URL (a `file:` URL made by the protocol setter from `ws:a/a|` keeps the
/// path `/a|`, which a parse reads as the drive letter `/a:`).
#[ephyra::test(gen = cases(), tests = 3000)]
fn href_round_trips(case: Case) {
    for kind in [Kind::Url, Kind::Aggregator] {
        let (t, _) = ada::run(kind, &case, 0);
        for (k, step) in t.steps.iter().enumerate() {
            let Some(url) = &step.url else { continue };
            let again = Case::parse(url.href.clone());
            let (mine, _) = ada::run(kind, &again, 0);
            if mine.steps[0].url.as_ref() != Some(url)
                && reference::run(&again).steps[0].url.as_ref() == Some(url)
            {
                assert_eq!(mine.steps[0].url.as_ref(), Some(url), "{kind:?}, step {k}");
            }
        }
    }
}

/// A URL behaves as the URL its href parses to: after every step, the
/// remaining setters do the same to the parsed href. ada keeps state beside
/// the href (`has_opaque_path`, offsets); this catches state that drifts from
/// what the href says.
#[ephyra::test(gen = cases(), tests = 3000)]
fn urls_behave_as_their_href(case: Case) {
    for kind in [Kind::Url, Kind::Aggregator] {
        let (t, _) = ada::run(kind, &case, 0);
        for (k, step) in t.steps.iter().enumerate() {
            let Some(url) = &step.url else { continue };
            // Hrefs the standard itself does not round-trip are
            // `href_round_trips`'s business.
            let again = Case { input: url.href.clone(), base: None, sets: case.sets[k..].to_vec() };
            let (mine, _) = ada::run(kind, &again, 0);
            if mine.steps[0].url.as_ref() != Some(url) {
                continue;
            }
            assert_eq!(
                mine.steps[1..],
                t.steps[k + 1..],
                "{kind:?}, from step {k}, href {:?}",
                url.href
            );
        }
    }
}

/// A setter that fails leaves the URL as it was.
#[ephyra::test(gen = cases(), tests = 3000)]
fn failed_setters_change_nothing(case: Case) {
    for kind in [Kind::Url, Kind::Aggregator] {
        let (t, _) = ada::run(kind, &case, 0);
        for k in 1..t.steps.len() {
            if !t.steps[k].ok {
                assert_eq!(t.steps[k].url, t.steps[k - 1].url, "{kind:?}, setter {}", k - 1);
            }
        }
    }
}

/// ada and whatwg-url, the standard's reference implementation, agree on
/// `canParse` and on the URL after every step (see `may_be_bidi_domain` for
/// the known gap).
#[ephyra::test(gen = cases(), tests = 3000)]
fn ada_agrees_with_reference(case: Case) {
    let (agg, _) = ada::run(Kind::Aggregator, &case, 0);
    let reference = reference::run(&case);
    if (agg.can_parse, urls(&agg)) != (reference.can_parse, urls(&reference))
        && may_be_bidi_domain(&case)
    {
        return;
    }
    assert_eq!(agg.can_parse, reference.can_parse, "canParse");
    assert_eq!(urls(&agg), urls(&reference));
}

/// Under `ada::set_max_input_length(limit)`, with a limit near the length of
/// one of the hrefs the case reaches (`at` picks it, `delta` moves the limit
/// around it): no URL is ever longer than the limit; a parse succeeds as
/// without a limit or fails because something was too long; and while the
/// limited URL is the unlimited one, a setter does what it does without a
/// limit when the result fits, and otherwise fails and changes nothing.
#[ephyra::test(gen = (cases(), ephyra::gens::int(0usize..8), ephyra::gens::int(-40i32..=40)), tests = 3000)]
fn length_limit_holds(case: Case, at: usize, delta: i32) {
    for kind in [Kind::Url, Kind::Aggregator] {
        let (free, _) = ada::run(kind, &case, 0);
        let lens: Vec<usize> = free.steps.iter().filter_map(|s| s.url.as_ref()).map(|u| u.href.len()).collect();
        if lens.is_empty() {
            continue;
        }
        let limit = (lens[at % lens.len()] as i64 + i64::from(delta)).max(1) as usize;
        let (limited, _) = ada::run(kind, &case, limit as u32);
        assert_eq!(limited.can_parse, limited.steps[0].ok, "{kind:?}: can_parse under the limit {limit}");
        for (k, step) in limited.steps.iter().enumerate() {
            if let Some(u) = &step.url {
                assert!(u.href.len() <= limit, "{kind:?}, step {k}: an href of {} bytes under the limit {limit}", u.href.len());
            }
        }
        let fits = |s: &Step| s.ok && s.url.as_ref().is_some_and(|u| u.href.len() <= limit);
        // The base is parsed under the limit too: as written or normalized,
        // it may not fit.
        let base_too_long = case.base.as_ref().is_some_and(|b| {
            let (t, _) = ada::run(kind, &Case::parse(b.clone()), 0);
            b.len() > limit || t.steps[0].url.as_ref().is_some_and(|u| u.href.len() > limit)
        });
        let too_long = case.input.len() > limit || base_too_long;
        if limited.steps[0].ok {
            assert_eq!(limited.steps[0], free.steps[0], "{kind:?}, parse under the limit {limit}");
        } else {
            assert!(!fits(&free.steps[0]) || too_long, "{kind:?}: the parse fails under the limit {limit}, though its href fits");
        }
        for k in 1..limited.steps.len().min(free.steps.len()) {
            if limited.steps[k - 1].url != free.steps[k - 1].url {
                break;
            }
            let (setter, value) = &case.sets[k - 1];
            // set_href parses, and a parse rejects an input longer than the
            // limit even when its normalized href would fit.
            if *setter == Setter::Href && value.len() > limit && !limited.steps[k].ok {
                assert_eq!(limited.steps[k].url, free.steps[k - 1].url, "{kind:?}, setter {}", k - 1);
                break;
            }
            if fits(&free.steps[k]) || !free.steps[k].ok {
                assert_eq!(limited.steps[k], free.steps[k], "{kind:?}, setter {} under the limit {limit}", k - 1);
            } else {
                // ada's set_search and set_hash return nothing: the shim
                // reports them as succeeding.
                let void = matches!(setter, Setter::Search | Setter::Hash);
                assert_eq!(
                    limited.steps[k],
                    Step { ok: void, url: free.steps[k - 1].url.clone() },
                    "{kind:?}, setter {}: its result does not fit the limit {limit}",
                    k - 1
                );
            }
        }
    }
}

/// ada's state predicates (`has_credentials`, `has_hostname`, `has_port`,
/// `has_search`, `has_hash` and the others, and `get_pathname_length`)
/// answer as the components and the href do, after the parse and after
/// every setter, in both representations. By the standard's serializer:
/// the href has `//` after the scheme exactly when the host is not null, a
/// `?` before any `#` exactly when the query is not null (an empty query
/// included), a `#` exactly when the fragment is not null; credentials,
/// port and the path are as their getters show them.
#[ephyra::test(gen = cases(), tests = 3000)]
fn predicates_agree_with_components(case: Case) {
    for kind in [Kind::Url, Kind::Aggregator] {
        let (t, _) = ada::run(kind, &case, 0);
        let urls: Vec<&Parts> = t.steps.iter().filter_map(|s| s.url.as_ref()).collect();
        let flags = ada::flags(kind, &case);
        assert_eq!(flags.len(), urls.len(), "{kind:?}: steps");
        for (k, (f, u)) in flags.iter().zip(urls).enumerate() {
            let at = format!("{kind:?}, step {k}, href {:?}", u.href);
            let host = u.href[u.protocol.len()..].starts_with("//");
            assert_eq!(f.has_hostname, host, "{at}: has_hostname");
            assert_eq!(f.has_empty_hostname, host && u.hostname.is_empty(), "{at}: has_empty_hostname");
            assert_eq!(f.has_search, u.href.split('#').next().unwrap().contains('?'), "{at}: has_search");
            assert_eq!(f.has_hash, u.href.contains('#'), "{at}: has_hash");
            assert_eq!(f.has_port, !u.port.is_empty(), "{at}: has_port");
            assert_eq!(f.has_credentials, !u.username.is_empty() || !u.password.is_empty(), "{at}: has_credentials");
            if let Some(b) = f.has_non_empty_username {
                assert_eq!(b, !u.username.is_empty(), "{at}: has_non_empty_username");
            }
            if let Some(b) = f.has_non_empty_password {
                assert_eq!(b, !u.password.is_empty(), "{at}: has_non_empty_password");
            }
            assert_eq!(f.pathname_length, u.pathname.len(), "{at}: get_pathname_length");
        }
    }
}

/// `ada::url` and `ada::url_aggregator` agree under `set_max_input_length`
/// too: with a limit near the length of one of the hrefs the case reaches
/// (`at` picks it, `delta` moves the limit around it), both representations
/// give the same transcript. The limit paths (rollback snapshots, length
/// checks) are written separately in the two representations, so a
/// difference is a bug in one of them.
#[ephyra::test(gen = (cases(), ephyra::gens::int(0usize..8), ephyra::gens::int(-40i32..=40)), tests = 3000)]
fn representations_agree_under_limits(case: Case, at: usize, delta: i32) {
    let (free, _) = ada::run(Kind::Aggregator, &case, 0);
    let lens: Vec<usize> = free.steps.iter().filter_map(|s| s.url.as_ref()).map(|u| u.href.len()).collect();
    if lens.is_empty() {
        return;
    }
    let limit = (lens[at % lens.len()] as i64 + i64::from(delta)).max(1) as u32;
    let (url, _) = ada::run(Kind::Url, &case, limit);
    let (agg, valid) = ada::run(Kind::Aggregator, &case, limit);
    assert!(valid, "url_aggregator::validate() failed under the limit {limit}");
    assert_eq!(url, agg, "under the limit {limit}");
}

/// ada's URLSearchParams agrees with whatwg-url's after the constructor and
/// every operation: results, size, serialization and entries.
#[ephyra::test(gen = ephyra_ada::params::params_cases(), tests = 3000)]
fn search_params_agree_with_reference(case: ephyra_ada::params::ParamsCase) {
    let ada = ephyra_ada::params::ada(&case);
    let reference = ephyra_ada::params::reference(&case);
    for (k, (a, r)) in ada.iter().zip(&reference).enumerate() {
        if a != r && declined_raw_bytes(a, r) {
            // Known and declined upstream (ada-url/ada#1279, "the user is
            // responsible for validating the correctness of the input"):
            // ada keeps the bytes of a percent escape that decodes to
            // ill-formed UTF-8, where the standard decodes them to U+FFFD.
            // Later steps diverge for the same reason (`%aa` and `%ab` are
            // two keys in ada, one in the standard), so stop comparing here.
            return;
        }
        assert_eq!(a, r, "step {k}");
    }
    assert_eq!(ada.len(), reference.len());
}

/// Whether the difference between ada's step `a` and the reference's `r` is
/// only #1279: decoding ada's raw bytes (shown as `\xNN` by `params::text`)
/// with replacement, as the standard does, makes the results, size and
/// entries equal. Rust's lossy decoding replaces maximal ill-formed
/// subsequences the way the Encoding Standard's UTF-8 decoder does.
fn declined_raw_bytes(a: &ephyra_ada::params::ParamsStep, r: &ephyra_ada::params::ParamsStep) -> bool {
    fn decode(s: &str) -> String {
        let mut bytes = Vec::new();
        let mut rest = s;
        while let Some(at) = rest.find("\\x") {
            bytes.extend_from_slice(rest[..at].as_bytes());
            match u8::from_str_radix(rest.get(at + 2..at + 4).unwrap_or(""), 16) {
                Ok(b) => {
                    bytes.push(b);
                    rest = &rest[at + 4..];
                },
                Err(_) => {
                    bytes.extend_from_slice(b"\\x");
                    rest = &rest[at + 2..];
                },
            }
        }
        bytes.extend_from_slice(rest.as_bytes());
        String::from_utf8_lossy(&bytes).into_owned()
    }
    let all = |v: &[String]| v.iter().map(|s| decode(s)).collect::<Vec<_>>();
    a.entries.iter().any(|e| e.contains("\\x"))
        && a.size == r.size
        && all(&a.entries) == r.entries
        && all(&a.result) == r.result
}

/// ada's URLPattern list answers as its routes do one by one: the winner
/// matches alone, with the same group values; a pathname some route
/// matches has a winner; and among routes of the plain static / `:param` /
/// `*` subset, the winner outranks every other route that matches (smaller
/// kind sequence, literal < `:param` < `*`, then fewer segments, then
/// insertion order), as `url_pattern_list.h` documents. A winner with more
/// than `max_captures_per_route` (8) captures reports the first 8 and sets
/// `captures_truncated`, which no other answer sets.
#[ephyra::test(gen = ephyra_ada::pattern_list::list_cases(), tests = 3000)]
fn pattern_list_agrees_with_patterns(case: ephyra_ada::pattern_list::ListCase) {
    use ephyra_ada::pattern_list::{plain_kinds, run};
    let r = run(&case);
    if !r.list_ok {
        return;
    }
    for p in &r.paths {
        let matching: Vec<usize> = (0..case.patterns.len()).filter(|&k| p.alone[k].is_some()).collect();
        match p.route {
            None => assert!(matching.is_empty(), "{:?}: no winner, though routes {matching:?} match alone", p.canonical),
            Some(w) => {
                let alone = p.alone[w].as_ref();
                assert!(alone.is_some(), "{:?}: route {w} wins but does not match alone", p.canonical);
                if p.truncated {
                    let alone = alone.unwrap();
                    assert!(!p.regexp && alone.len() > 8, "{:?}: route {w} truncated, {} values alone", p.canonical, alone.len());
                    assert_eq!(p.values[..], alone[..8], "{:?}: truncated values of route {w}", p.canonical);
                } else {
                    assert_eq!(Some(&p.values), alone, "{:?}: values of route {w}", p.canonical);
                }
                if let Some(wk) = plain_kinds(&case.patterns[w]) {
                    for &k in &matching {
                        let Some(kk) = plain_kinds(&case.patterns[k]) else { continue };
                        assert!(
                            (&wk, w) <= (&kk, k),
                            "{:?}: route {w} {:?} wins over route {k} {:?}",
                            p.canonical,
                            case.patterns[w],
                            case.patterns[k]
                        );
                    }
                }
            },
        }
    }
}

/// Whether `u`'s hostname comes back unchanged from `https://example.com`'s
/// hostname setter: true for every special URL's host, false for opaque
/// hosts the special-host parser changes or rejects.
fn survives_as_special_host(u: &Parts) -> bool {
    if u.hostname.is_empty() {
        return true;
    }
    let set = Case { input: "https://example.com".to_owned(), base: None, sets: vec![(Setter::Hostname, u.hostname.clone())] };
    let (t, _) = ada::run(Kind::Aggregator, &set, 0);
    t.steps[1].ok && t.steps[1].url.as_ref().is_some_and(|v| v.hostname == u.hostname)
}

/// ada's URLPattern answers as urlpattern-polyfill and Deno's `urlpattern`
/// do whenever those two agree with each other: whether the pattern
/// compiles, its components, and whether each input matches. The two
/// references disagree with each other and with ada on many cases (each
/// has its own bugs and its own URL parser), so a two-way differential is
/// noise; two independent implementations agreeing against ada is not.
/// Not compared:
/// - ada alone compiling a pattern both reject (the modifier on an escaped
///   delimiter, `\:*`, an open question; see README.md);
/// - cases with an input std::regex gave up on (answer 3);
/// - inputs whose pathname starts with `//` (`file:////users/`): the
///   polyfill re-parses the pathname as a whole URL, making `//users` a
///   host (kenchris/urlpattern-polyfill#128), and the `url` crate under
///   Deno drops the empty segments of a `file:` path; ada, as the URL
///   Standard, keeps them;
/// - inputs with an opaque host that the special-host parser would change
///   or reject (`%C3%A9`, `2`, `.1`): the polyfill canonicalizes an input's
///   hostname by setting it on an `https:` URL, so `foo://2/` has the
///   hostname `0.0.0.2` there and `foo://é/` none at all;
/// - under `ignoreCase`, inputs whose answer in ada depends on it: Deno's
///   `urlpattern` compares a component's literal prefix and suffix case
///   sensitively (`/*a9` does not match `/xA9`).
#[ephyra::test(gen = ephyra_ada::pattern::pattern_cases(), tests = 3000)]
fn pattern_agrees_with_reference_majority(case: ephyra_ada::pattern::PatternCase) {
    use ephyra_ada::pattern::{ada, deno, reference_within, PatternResult};
    let a = ada(&case);
    // A regex std::regex gives up on (a nested quantifier) also backtracks
    // for minutes in the polyfill's JS RegExp: skip the case before asking.
    if a.inputs.iter().any(|i| i.exec == 3 || i.test == 3) {
        return;
    }
    let Some(d) = deno(&case) else { return };
    // The polyfill backtracks for minutes on some regexes std::regex runs
    // fast: no answer within the time, no vote.
    let Some(p) = reference_within(&case, std::time::Duration::from_secs(2)) else { return };
    let folded = case.ignore_case.then(|| ada(&ephyra_ada::pattern::PatternCase { ignore_case: false, ..case.clone() }));
    let skipped: Vec<bool> = case
        .inputs
        .iter()
        .enumerate()
        .map(|(k, (input, base))| {
            let case_matters = folded.as_ref().is_some_and(|f| f.inputs.get(k).map(|i| i.exec) != a.inputs.get(k).map(|i| i.exec));
            let (t, _) = ada::run(Kind::Aggregator, &Case { input: input.clone(), base: base.clone(), sets: Vec::new() }, 0);
            case_matters
                || t.steps[0].url.as_ref().is_some_and(|u| u.pathname.starts_with("//") || !survives_as_special_host(u))
        })
        .collect();
    let side = |r: &PatternResult| {
        let execs = r.inputs.iter().zip(&skipped).filter(|(_, s)| !**s).map(|(i, _)| i.exec);
        (r.ok, r.components.clone(), execs.collect::<Vec<_>>())
    };
    if side(&p) != side(&d) || side(&a) == side(&p) || (a.ok && !p.ok) {
        return;
    }
    assert_eq!(side(&a), side(&p), "ada vs (polyfill = deno)");
}

/// The same for a URLPatternInit input (a dictionary, not a URL string),
/// on the all-wildcard pattern `{}`: `test` answers whether `exec` matches,
/// and with a base URL string both throw a TypeError, as the standard says
/// for a dictionary input given a base.
#[ephyra::test(gen = ephyra_ada::init_inputs::init_cases(), tests = 3000)]
fn init_test_agrees_with_exec(inputs: Vec<ephyra_ada::init_inputs::InitInput>) {
    for (k, [test, exec, test_base, exec_base]) in ephyra_ada::init_inputs::ada_test(&inputs).into_iter().enumerate() {
        assert_eq!(test, exec, "input {k}: test vs exec");
        assert_eq!((test_base, exec_base), (2, 2), "input {k}: with a base string");
    }
}

/// URLPattern's `test` answers whether `exec` matches, and both fail on the
/// same inputs.
#[ephyra::test(gen = ephyra_ada::pattern::pattern_cases(), tests = 3000)]
fn pattern_test_agrees_with_exec(case: ephyra_ada::pattern::PatternCase) {
    let r = ephyra_ada::pattern::ada(&case);
    // 3: std::regex gave up on one of them (see `cpp/shim.cpp`).
    for (k, i) in r.inputs.iter().enumerate().filter(|(_, i)| i.test != 3 && i.exec != 3) {
        assert_eq!(i.test, i.exec, "input {k}: test vs exec");
    }
}

/// Component strings are stable after one pass: when a pattern made from a
/// compiled pattern's components (as an init, same `ignoreCase`) compiles,
/// a pattern made from that one's components has the same components
/// and answers every input the same. (One pass is allowed to change them:
/// in `HTTPS::443` the uppercase scheme keeps the default port, which the
/// lowercased components then drop, in every implementation.)
#[ephyra::test(gen = ephyra_ada::pattern::pattern_cases(), tests = 3000)]
fn pattern_components_are_stable(case: ephyra_ada::pattern::PatternCase) {
    use ephyra_ada::pattern::{ada, PatternCase, PatternResult, Spec};
    let rebuild = |r: &PatternResult| {
        let mut fields: [Option<String>; 9] = Default::default();
        for (f, c) in fields.iter_mut().zip(&r.components) {
            *f = Some(c.clone());
        }
        ada(&PatternCase { spec: Spec::Init(fields), base: None, ..case.clone() })
    };
    let r = ada(&case);
    if !r.ok {
        return;
    }
    let once = rebuild(&r);
    // A constructor string can yield components that do not compile alone
    // (`([]):/.{/}/]` gives the pathname `//]`), in ada and the polyfill
    // alike; only components that compile are held to stability.
    if !once.ok {
        return;
    }
    let twice = rebuild(&once);
    assert!(twice.ok, "the components {:?} do not compile", once.components);
    // The spec itself is not stable everywhere (an init pathname `//` with
    // a non-special protocol canonicalizes to the empty string): when
    // urlpattern-polyfill, a transcription of the spec, rebuilds the same
    // components to the same result, the instability is the spec's.
    if twice.components != once.components {
        let mut fields: [Option<String>; 9] = Default::default();
        for (f, c) in fields.iter_mut().zip(&once.components) {
            *f = Some(c.clone());
        }
        let rebuilt = PatternCase { spec: Spec::Init(fields), base: None, ..case.clone() };
        // No answer within the time: the instability stays unexplained.
        let theirs = ephyra_ada::pattern::reference_within(&rebuilt, std::time::Duration::from_secs(2));
        if theirs.is_some_and(|t| t.ok && t.components == twice.components) {
            return;
        }
    }
    assert_eq!(twice.components, once.components, "components");
    assert_eq!(twice.inputs, once.inputs, "answers");
}



/// A URL ada canonicalizes to (its own href) is also whatwg-url's canonical
/// form: re-parsing it there gives canParse and the identical href. Both
/// implement the same standard, so a difference is a real bug. The input is
/// drawn from `canonical_urls`, a generator ephyra verifies produces only
/// strings with `parse(s).href == s`, so this samples the valid region
/// densely instead of mostly hitting parse failures.
#[ephyra::test(gen = ephyra_ada::canonical::canonical_urls(), tests = 3000)]
fn canonical_url_is_reference_canonical(url: String) {
    let r = reference::run(&Case::parse(url.clone()));
    assert!(r.can_parse, "whatwg-url rejects ada's canonical href {url:?}");
    assert_eq!(r.steps[0].url.as_ref().map(|u| u.href.as_str()), Some(url.as_str()));
}

/// Applying a setter with a value twice equals applying it once: ada's
/// setters are idempotent. Sound for every component (no empty-delimiter
/// asymmetry, since both sides use the same value, not a getter). Needs a
/// URL that parses, which `cases()` supplies with its setter calls.
#[ephyra::test(gen = cases(), tests = 3000)]
fn setters_are_idempotent(case: Case) {
    let Some((setter, value)) = case.sets.first().cloned() else { return };
    let once = Case { input: case.input.clone(), base: case.base.clone(), sets: vec![(setter, value.clone())] };
    let twice = Case { input: case.input.clone(), base: case.base.clone(), sets: vec![(setter, value.clone()), (setter, value.clone())] };
    for kind in [Kind::Url, Kind::Aggregator] {
        let (a, _) = ada::run(kind, &once, 0);
        if a.steps[0].url.is_none() {
            continue;
        }
        let (b, _) = ada::run(kind, &twice, 0);
        assert_eq!(a.steps[1].url, b.steps[2].url, "{kind:?}, {setter:?}={value:?}");
    }
}
