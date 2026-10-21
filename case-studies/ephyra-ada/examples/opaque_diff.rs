//! Non-special pathname canonicalization: ada (vendored, with fixes) vs
//! Deno's `urlpattern`, over generated values starting with "//". Prints
//! differences, split by whether the value ends in a space or C0 control
//! (stock ada trims those, Deno does not: a separate question).
use ephyra::Source;
use ephyra_ada::pattern::{ada, deno, PatternCase, PatternResult, Spec};

fn main() {
    let pieces = [
        "/",
        "//",
        "a",
        "b.c",
        " ",
        "\u{1}",
        "\u{7f}",
        "é",
        "%",
        "%41",
        "\u{1F600}",
        "~",
        "-",
        "=",
        "&",
        "!",
    ];
    let (mut compared, mut trailing, mut other) = (0, 0, 0);
    for i in 0..30_000u64 {
        let mut g = Source::new(i, 16);
        let mut path = String::from("//");
        for _ in 0..g.int(0u32..6) {
            path.push_str(g.pick(&pieces));
        }
        let mut f: [Option<String>; 9] = Default::default();
        f[0] = Some((*g.pick(&["foo", "data", "x-y"])).to_owned());
        f[5] = Some(path.clone());
        let case =
            PatternCase { spec: Spec::Init(f), base: None, ignore_case: false, inputs: vec![] };
        let Some(d) = deno(&case) else { continue };
        let a = ada(&case);
        let pn = |r: &PatternResult| r.ok.then(|| r.components[5].clone());
        compared += 1;
        if pn(&a) == pn(&d) {
            continue;
        }
        if path.ends_with(|c: char| c <= ' ') {
            trailing += 1;
        } else {
            other += 1;
            if other <= 10 {
                println!("pathname={path:?}  ada={:?}  deno={:?}", pn(&a), pn(&d));
            }
        }
    }
    println!("{compared} compared: {other} differ, plus {trailing} that differ only with a trailing space or control");
}
