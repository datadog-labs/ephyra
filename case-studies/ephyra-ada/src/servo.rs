//! Cases run on the `url` crate (servo's), through its `quirks` module, the
//! standard's `URL` interface.

use url::{quirks, Url};

use crate::{Case, Parts, Setter, Step, Transcript};

fn parts(u: &Url) -> Parts {
    Parts {
        href: quirks::href(u).to_owned(),
        origin: quirks::origin(u),
        protocol: quirks::protocol(u).to_owned(),
        username: quirks::username(u).to_owned(),
        password: quirks::password(u).to_owned(),
        host: quirks::host(u).to_owned(),
        hostname: quirks::hostname(u).to_owned(),
        port: quirks::port(u).to_owned(),
        pathname: quirks::pathname(u).to_owned(),
        search: quirks::search(u).to_owned(),
        hash: quirks::hash(u).to_owned(),
    }
}

/// The transcript of `case` in the `url` crate. Its `pathname`, `search`
/// and `hash` setters report nothing; they count as succeeding.
pub fn run(case: &Case) -> Transcript {
    let parsed = match &case.base {
        None => Url::parse(&case.input).ok(),
        Some(base) => Url::parse(base).ok().and_then(|b| b.join(&case.input).ok()),
    };
    let Some(mut u) = parsed else {
        return Transcript { can_parse: false, steps: vec![Step { ok: false, url: None }] };
    };
    let mut steps = vec![Step { ok: true, url: Some(parts(&u)) }];
    for (setter, v) in &case.sets {
        let ok = match setter {
            Setter::Href => quirks::set_href(&mut u, v).is_ok(),
            Setter::Protocol => quirks::set_protocol(&mut u, v).is_ok(),
            Setter::Username => quirks::set_username(&mut u, v).is_ok(),
            Setter::Password => quirks::set_password(&mut u, v).is_ok(),
            Setter::Host => quirks::set_host(&mut u, v).is_ok(),
            Setter::Hostname => quirks::set_hostname(&mut u, v).is_ok(),
            Setter::Port => quirks::set_port(&mut u, v).is_ok(),
            Setter::Pathname => {
                quirks::set_pathname(&mut u, v);
                true
            },
            Setter::Search => {
                quirks::set_search(&mut u, v);
                true
            },
            Setter::Hash => {
                quirks::set_hash(&mut u, v);
                true
            },
        };
        steps.push(Step { ok, url: Some(parts(&u)) });
    }
    Transcript { can_parse: true, steps }
}
