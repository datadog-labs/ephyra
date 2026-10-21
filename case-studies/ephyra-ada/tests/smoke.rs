//! ada's two representations and the `url` crate agree on a few cases.

use ephyra_ada::{
    ada::{self, Kind},
    servo, Case, Setter,
};

fn check(case: &Case) {
    let (url, url_valid) = ada::run(Kind::Url, case, 0);
    let (agg, agg_valid) = ada::run(Kind::Aggregator, case, 0);
    assert!(url_valid && agg_valid, "{case:?}");
    assert_eq!(url, agg, "{case:?}");
    let servo = servo::run(case);
    let parts =
        |t: &ephyra_ada::Transcript| t.steps.iter().map(|s| s.url.clone()).collect::<Vec<_>>();
    assert_eq!(parts(&agg), parts(&servo), "{case:?}");
}

#[test]
fn ada_and_url_agree() {
    for input in [
        "https://user:pass@example.com:8080/a/./b/../c?q=1#f",
        "HTTP://EXAMPLE.COM/%7Efoo",
        "http://0x7f.1/",
        "http://[::ffff:192.168.0.1]/",
        "https://xn--nxasmq6b.com/",
        "https://\u{3b2}\u{3cc}\u{3bb}\u{3bf}\u{3c2}.com/",
        "file:///C:/dir/../file",
        "  \thttp://ex\nample.com\\path ",
        "non-special://host/a b",
        "data:text/plain,hello",
        "http://[::1",
        "http://exa mple.com/",
    ] {
        check(&Case::parse(input));
    }
    check(&Case { input: "../x?y".into(), base: Some("http://h/a/b/c".into()), sets: vec![] });
    check(&Case {
        input: "https://example.com/".into(),
        base: None,
        sets: vec![
            (Setter::Port, "8080".into()),
            (Setter::Username, "u s".into()),
            (Setter::Pathname, "/a/../b c".into()),
            (Setter::Protocol, "http".into()),
            (Setter::Hash, "#x".into()),
            (Setter::Host, "[::1]:99".into()),
        ],
    });
}

#[test]
fn reference_agrees_on_a_setter_chain() {
    let case = Case {
        input: "https://example.com/".into(),
        base: None,
        sets: vec![
            (Setter::Port, "8080".into()),
            (Setter::Pathname, "/a/../b c".into()),
            (Setter::Href, "::".into()),
        ],
    };
    let (agg, _) = ada::run(Kind::Aggregator, &case, 0);
    let reference = ephyra_ada::reference::run(&case);
    assert_eq!(reference.steps.len(), 4);
    assert!(!reference.steps[3].ok);
    assert_eq!(
        agg.steps.iter().map(|s| &s.url).collect::<Vec<_>>(),
        reference.steps.iter().map(|s| &s.url).collect::<Vec<_>>()
    );
    // The url crate's disagreement found by the first run: whatwg-url sides
    // with ada.
    let backslash = Case { input: "\\".into(), base: Some("a:/".into()), sets: vec![] };
    let (agg, _) = ada::run(Kind::Aggregator, &backslash, 0);
    assert_eq!(agg.steps[0].url, ephyra_ada::reference::run(&backslash).steps[0].url);
}

/// ../bugs/ada/ada-file-drive-letter-opaque-path: a drive-letter reference
/// against a `file:` base must not leave the URL with an opaque path. Holds
/// with the patch `vendor/COMMIT` names; fails on an unpatched ada.
#[test]
fn drive_letter_reference_against_file_base_is_not_opaque() {
    let case = Case {
        input: "C|".into(),
        base: Some("file:".into()),
        sets: vec![(Setter::Hostname, "h".into()), (Setter::Pathname, "/D:/y".into())],
    };
    for kind in [Kind::Url, Kind::Aggregator] {
        let (t, _) = ada::run(kind, &case, 0);
        let hrefs: Vec<_> =
            t.steps.iter().map(|s| (s.ok, s.url.as_ref().unwrap().href.as_str())).collect();
        assert_eq!(
            hrefs,
            [(true, "file:///C:"), (true, "file://h/C:"), (true, "file://h/D:/y")],
            "{kind:?}"
        );
    }
}
