// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Cases run on ada through `cpp/shim.cpp`.

use std::sync::Mutex;

use crate::{Case, Parts, Setter, Step, Transcript};

/// One of ada's two URL representations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `ada::url`: each component a string of its own.
    Url = 0,
    /// `ada::url_aggregator`: the href in one buffer, components as offsets.
    Aggregator = 1,
}

extern "C" {
    fn ea_run(
        kind: i32,
        bytes: *const u8,
        len: usize,
        max_input_length: u32,
        out: *mut *mut u8,
    ) -> usize;
    fn ea_free(p: *mut u8);
}

/// `ada::set_max_input_length` is process-global; runs (and search-params
/// runs, which read it) hold this so a limit set for one never applies to
/// another.
pub(crate) static LIMIT: Mutex<()> = Mutex::new(());

/// Runs `f` holding [`LIMIT`]: every call into the shim does, since all of
/// ada's parsing reads the process-global limit.
pub(crate) fn locked<T>(f: impl FnOnce() -> T) -> T {
    let _limit = LIMIT.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// What running `case` on `kind` gives: the transcript, and whether
/// `url_aggregator::validate()` held after every step (always, for
/// `ada::url`). `max_input_length` 0 keeps ada's default limit.
pub fn run(kind: Kind, case: &Case, max_input_length: u32) -> (Transcript, bool) {
    decode(&call(kind as i32, case, max_input_length))
}

/// The shim's output for `case` in `mode` (see `ea_run`).
fn call(mode: i32, case: &Case, max_input_length: u32) -> Vec<u8> {
    let bytes = encode(case);
    let mut out = std::ptr::null_mut();
    let len = {
        let _limit = LIMIT.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: `bytes` is live for the call; the shim writes a malloc'd
        // buffer of `len` bytes to `out`, freed below.
        unsafe { ea_run(mode, bytes.as_ptr(), bytes.len(), max_input_length, &mut out) }
    };
    // SAFETY: as above; copied before it is freed.
    let transcript = unsafe { std::slice::from_raw_parts(out, len) }.to_vec();
    unsafe { ea_free(out) };
    transcript
}

/// A URL's state predicates, as ada answers them (`has_credentials()` and
/// so on). The non-empty username and password predicates exist only on
/// `ada::url_aggregator` (`None` on `ada::url`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub has_credentials: bool,
    pub has_empty_hostname: bool,
    pub has_hostname: bool,
    pub has_port: bool,
    pub has_search: bool,
    pub has_hash: bool,
    pub has_non_empty_username: Option<bool>,
    pub has_non_empty_password: Option<bool>,
    pub pathname_length: usize,
}

/// The state predicates after each step of `case` that leaves a URL, in
/// the order of [`run`]'s steps (none for a failed parse).
pub fn flags(kind: Kind, case: &Case) -> Vec<Flags> {
    let t = call(kind as i32 + 2, case, 0);
    let mut r = &t[1..];
    let mut out = Vec::new();
    while let Some((&ok, rest)) = r.split_first() {
        if out.is_empty() && ok == 0 {
            break;
        }
        let b = |i: usize| match rest[i] {
            2 => None,
            v => Some(v == 1),
        };
        out.push(Flags {
            has_credentials: rest[0] == 1,
            has_empty_hostname: rest[1] == 1,
            has_hostname: rest[2] == 1,
            has_port: rest[3] == 1,
            has_search: rest[4] == 1,
            has_hash: rest[5] == 1,
            has_non_empty_username: b(6),
            has_non_empty_password: b(7),
            pathname_length: u32::from_le_bytes(rest[8..12].try_into().unwrap()) as usize,
        });
        r = &rest[12..];
    }
    out
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn encode(case: &Case) -> Vec<u8> {
    let mut out = Vec::new();
    put_str(&mut out, &case.input);
    match &case.base {
        Some(base) => {
            out.push(1);
            put_str(&mut out, base);
        },
        None => out.push(0),
    }
    out.extend_from_slice(&(case.sets.len() as u32).to_le_bytes());
    for (setter, value) in &case.sets {
        let index = Setter::ALL.iter().position(|s| s == setter).unwrap();
        out.push(index as u8);
        put_str(&mut out, value);
    }
    out
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn str(&mut self) -> String {
        let n = u32::from_le_bytes(self.0[..4].try_into().unwrap()) as usize;
        let s = &self.0[4..4 + n];
        self.0 = &self.0[4 + n..];
        // Components are ASCII by the standard; a lossy decode keeps any
        // other byte visible as a difference instead of a panic.
        String::from_utf8_lossy(s).into_owned()
    }

    fn parts(&mut self) -> Parts {
        Parts {
            href: self.str(),
            origin: self.str(),
            protocol: self.str(),
            username: self.str(),
            password: self.str(),
            host: self.str(),
            hostname: self.str(),
            port: self.str(),
            pathname: self.str(),
            search: self.str(),
            hash: self.str(),
        }
    }
}

fn decode(bytes: &[u8]) -> (Transcript, bool) {
    let mut r = Reader(bytes);
    let can_parse = r.byte() == Some(1);
    let mut steps = Vec::new();
    let mut valid = true;
    while let Some(ok) = r.byte() {
        // A failed parse is the last step and has no URL; a failed setter
        // leaves one.
        if steps.is_empty() && ok == 0 {
            steps.push(Step { ok: false, url: None });
            break;
        }
        valid &= r.byte() == Some(1);
        steps.push(Step { ok: ok == 1, url: Some(r.parts()) });
    }
    (Transcript { can_parse, steps }, valid)
}
