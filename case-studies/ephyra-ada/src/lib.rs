//! A testing engine for ada, the WHATWG URL parser, built on ephyra. A
//! [`Case`] is an input, an optional base and setter calls; [`ada::run`]
//! runs it on either of ada's URL representations and [`servo::run`] on the
//! `url` crate, each returning a [`Transcript`] to compare. `README.md` has
//! the design and the findings.

pub mod ada;
pub mod canonical;
pub mod gen;
pub mod idna_direct;
pub mod init_inputs;
pub mod params;
pub mod pattern;
pub mod pattern_list;
pub mod reference;
pub mod servo;

/// A URL setter, as the standard's `URL` interface has them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Setter {
    Href,
    Protocol,
    Username,
    Password,
    Host,
    Hostname,
    Port,
    Pathname,
    Search,
    Hash,
}

impl Setter {
    pub const ALL: [Setter; 10] = [
        Setter::Href,
        Setter::Protocol,
        Setter::Username,
        Setter::Password,
        Setter::Host,
        Setter::Hostname,
        Setter::Port,
        Setter::Pathname,
        Setter::Search,
        Setter::Hash,
    ];
}

/// `new URL(input, base)`, then each setter in turn.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Case {
    pub input: String,
    pub base: Option<String>,
    pub sets: Vec<(Setter, String)>,
}

impl Case {
    pub fn parse(input: impl Into<String>) -> Case {
        Case { input: input.into(), base: None, sets: Vec::new() }
    }
}

/// A URL's getters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parts {
    pub href: String,
    pub origin: String,
    pub protocol: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub hostname: String,
    pub port: String,
    pub pathname: String,
    pub search: String,
    pub hash: String,
}

/// The parse or one setter call: whether it succeeded, and the URL after it
/// (none when the parse failed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub ok: bool,
    pub url: Option<Parts>,
}

/// What running a case did: `URL.canParse(input, base)`, then the parse step
/// and one step per setter (none after a failed parse).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub can_parse: bool,
    pub steps: Vec<Step>,
}
