//! Prints a URLPattern constructor string's components in ada and in
//! urlpattern-polyfill, for triage.
//!
//! cargo run --example show_pattern -- PATTERN [--base BASE] [INPUT]...

use ephyra_ada::pattern::{ada, reference, PatternCase, Spec, COMPONENTS};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut case = PatternCase {
        spec: Spec::Text(args.next().expect("a pattern")),
        base: None,
        ignore_case: false,
        inputs: vec![],
    };
    while let Some(a) = args.next() {
        if a == "--base" {
            case.base = args.next();
        } else {
            case.inputs.push((a, None));
        }
    }
    for (name, r) in [("ada     ", ada(&case)), ("polyfill", reference(&case))] {
        if !r.ok {
            println!("{name}: does not compile");
            continue;
        }
        let parts: Vec<String> =
            COMPONENTS.iter().zip(&r.components).map(|(k, v)| format!("{k}={v:?}")).collect();
        println!("{name}: {}", parts.join(" "));
        for (i, x) in r.inputs.iter().enumerate() {
            println!(
                "  input {i}: test {} exec {} {:?}",
                x.test,
                x.exec,
                x.groups.iter().filter(|g| !g.1.is_empty()).collect::<Vec<_>>()
            );
        }
    }
}
