//! ephyra-ada's properties (`tests/oracles.rs` includes them) on ada built
//! with AddressSanitizer, UBSan and libc++ hardening (`.cargo/config.toml`).
//! A failure of any of them aborts the process; the case being run is then
//! in `target/crash.bin`, which [`replay`] runs again.

use ephyra_ada::{
    ada::{self, Kind},
    Case, Setter,
};

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        let b = self.0[0];
        self.0 = &self.0[1..];
        b
    }

    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.0[..4].try_into().unwrap());
        self.0 = &self.0[4..];
        v
    }

    fn str(&mut self) -> String {
        let n = self.u32() as usize;
        let s = String::from_utf8_lossy(&self.0[..n]).into_owned();
        self.0 = &self.0[n..];
        s
    }
}

/// The case saved in `target/crash.bin` by the shim: kind, limit and the
/// case's bytes, in the shim's encoding.
pub fn read_crash(bytes: &[u8]) -> (Kind, u32, Case) {
    let mut r = Reader(bytes);
    let kind = if r.byte() == 0 { Kind::Url } else { Kind::Aggregator };
    let limit = r.u32();
    let input = r.str();
    let base = (r.byte() == 1).then(|| r.str());
    let sets = (0..r.u32()).map(|_| (Setter::ALL[usize::from(r.byte())], r.str())).collect();
    (kind, limit, Case { input, base, sets })
}

extern "C" {
    fn ea_params(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_pattern_list(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_pattern(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
}

/// Runs a saved case again: under the sanitizers, it fails the same way.
/// The first byte says which runner it was for: 0 and 1 a URL case on
/// `ada::url` or the aggregator, 2 search params, 3 a URLPattern list, 4 a
/// URLPattern.
pub fn replay(bytes: &[u8]) {
    let runner = match bytes[0] {
        2 => ea_params,
        3 => ea_pattern_list,
        4 => ea_pattern,
        _ => {
            let (kind, limit, case) = read_crash(bytes);
            eprintln!("replaying {kind:?}, limit {limit}: {case:?}");
            ada::run(kind, &case, limit);
            return;
        },
    };
    eprintln!("replaying runner {} on {} bytes", bytes[0], bytes.len() - 5);
    let body = &bytes[5..];
    let mut out = std::ptr::null_mut();
    // SAFETY: `body` is live for the call; the result is leaked, as the
    // process is expected to die in it.
    unsafe { runner(body.as_ptr(), body.len(), &mut out) };
}
