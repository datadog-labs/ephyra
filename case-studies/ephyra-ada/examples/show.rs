//! Prints a case's transcript in ada (both representations), whatwg-url and
//! the `url` crate, for triage.
//!
//! cargo run --example show -- INPUT [--base BASE] [SETTER=VALUE]...

use ephyra_ada::{
    ada::{self, Kind},
    reference, servo, Case, Setter, Transcript,
};

fn print(name: &str, t: &Transcript) {
    println!("{name}: canParse {}", t.can_parse);
    for (k, s) in t.steps.iter().enumerate() {
        match &s.url {
            Some(u) => println!("  step {k} ok {}: {u:?}", s.ok),
            None => println!("  step {k} failed"),
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut case = Case::parse(args.next().expect("an input"));
    while let Some(a) = args.next() {
        if a == "--base" {
            case.base = args.next();
            continue;
        }
        let (name, value) = a.split_once('=').expect("SETTER=VALUE");
        let setter = *Setter::ALL
            .iter()
            .find(|s| format!("{s:?}").eq_ignore_ascii_case(name))
            .expect("a setter name");
        case.sets.push((setter, value.to_owned()));
    }
    println!("{case:?}");
    let (url, _) = ada::run(Kind::Url, &case, 0);
    let (agg, valid) = ada::run(Kind::Aggregator, &case, 0);
    print("ada::url", &url);
    print(&format!("ada::url_aggregator (valid {valid})"), &agg);
    print("whatwg-url", &reference::run(&case));
    print("url crate", &servo::run(&case));
}
