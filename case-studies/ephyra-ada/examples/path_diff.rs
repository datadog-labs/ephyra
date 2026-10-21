//! Broad controlled differential of URLPattern special-scheme pathname
//! canonicalization: ada (vendored, with the fix) vs urlpattern-polyfill vs
//! Deno. Generates pathname patterns and compares the pathname component for
//! special protocols. Prints any case where ada differs from both
//! references (which agree with each other).
use ephyra::Source;
use ephyra_ada::pattern::{ada, deno, reference, PatternCase, Spec};

fn gen_path(g: &mut Source) -> String {
    let segs = ["a", "b", "", ".", "..", "x.y", "%2e", "c", "A"];
    let seps = ["/", "\\", "//", "/\\", "\\\\"];
    let mut s = String::new();
    for _ in 0..1 + (g.int(0u32..4) as usize) {
        s.push_str(g.pick(&seps));
        s.push_str(g.pick(&segs));
    }
    s
}

fn main() {
    let specials = ["https", "http", "ws", "wss", "ftp", "file"];
    let mut diffs = 0;
    for i in 0..40_000u64 {
        let mut gg = Source::new(i, 16);
        let proto = *gg.pick(&specials);
        let path = gen_path(&mut gg);
        let mut f: [Option<String>; 9] = Default::default();
        f[0] = Some(proto.to_owned());
        f[3] = Some("h".to_owned());
        f[5] = Some(path.clone());
        let case =
            PatternCase { spec: Spec::Init(f), base: None, ignore_case: false, inputs: vec![] };
        let a = ada(&case);
        let p = reference(&case);
        let Some(d) = deno(&case) else { continue };
        let pn =
            |r: &ephyra_ada::pattern::PatternResult| (r.ok, r.ok.then(|| r.components[5].clone()));
        if pn(&p) == pn(&d) && pn(&a) != pn(&p) {
            diffs += 1;
            if diffs <= 20 {
                println!("proto={proto:<5} path={path:?}  ada={:?} refs={:?}", pn(&a).1, pn(&p).1);
            }
        }
    }
    println!("{diffs} cases where ada differs from both references (which agree)");
}
