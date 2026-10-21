//! A URLPattern (and inputs) in ada, the polyfill and Deno's urlpattern:
//! components and exec per input, for triage.
//!
//! cargo run --release --example show_pattern3 -- PATTERN [INPUT]...
//!
//! `init:protocol=…|pathname=…` gives an init (fields named as in
//! URLPatternInit, `baseURL` included); anything else is a constructor
//! string. EA_IGNORE_CASE=1 sets ignoreCase.

use ephyra_ada::pattern::{ada, deno, reference, PatternCase, Spec};

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().expect("a pattern");
    let spec = match first.strip_prefix("init:") {
        Some(rest) => {
            let names = [
                "protocol", "username", "password", "hostname", "port", "pathname", "search",
                "hash", "baseURL",
            ];
            let mut fields: [Option<String>; 9] = Default::default();
            for kv in rest.split('|') {
                let (k, v) = kv.split_once('=').expect("field=value");
                fields[names.iter().position(|n| *n == k).expect("a field name")] =
                    Some(v.to_owned());
            }
            Spec::Init(fields)
        },
        None => Spec::Text(first),
    };
    let case = PatternCase {
        spec,
        base: None,
        ignore_case: std::env::var_os("EA_IGNORE_CASE").is_some(),
        inputs: args.map(|a| (a, None)).collect(),
    };
    let deno_r = deno(&case).unwrap_or(ephyra_ada::pattern::PatternResult {
        ok: false,
        components: vec![],
        regexp_groups: false,
        inputs: vec![],
    });
    for (n, r) in [("ada ", ada(&case)), ("pol ", reference(&case)), ("deno", deno_r)] {
        if r.ok {
            println!(
                "{n} {:?} exec={:?}",
                r.components,
                r.inputs.iter().map(|i| i.exec).collect::<Vec<_>>()
            );
        } else {
            println!("{n} does not compile");
        }
    }
}
