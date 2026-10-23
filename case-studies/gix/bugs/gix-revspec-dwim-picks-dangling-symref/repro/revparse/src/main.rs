// Usage: revparse <repo-dir> <spec>... ; prints the resolved id or the error per spec.
fn main() {
    let mut args = std::env::args().skip(1);
    let repo = gix::open(args.next().expect("repo dir")).expect("open repo");
    let mut failed = false;
    for spec in args {
        match repo.rev_parse_single(spec.as_str()) {
            Ok(id) => println!("{spec}: {id}"),
            Err(err) => {
                failed = true;
                let mut msg = err.to_string();
                let mut src = std::error::Error::source(&err);
                while let Some(s) = src {
                    msg.push_str(&format!(": {s}"));
                    src = s.source();
                }
                println!("{spec}: error: {msg}");
            }
        }
    }
    std::process::exit(failed as i32);
}
