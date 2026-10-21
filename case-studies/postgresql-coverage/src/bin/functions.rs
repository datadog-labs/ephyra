//! Write `functions.tsv` (function name, source file) from the instrumented
//! cluster's merged profile, starting the cluster first if needed.
//!
//! `cargo run --release --bin functions`

use ephyra_postgresql::pg;
use postgresql_coverage::{function_files, use_instrumented_cluster, write_function_files, Cluster, Profile};

fn main() {
    use_instrumented_cluster();
    pg::with_session(|s| s.fresh()).unwrap();
    pg::close_session();
    let c = Cluster::find().unwrap();
    let n = write_function_files(&c.profile).unwrap();
    let p = Profile::parse(&std::fs::read(&c.profile).unwrap()).unwrap();
    let files = function_files(&p).unwrap();
    let mapped = files.iter().filter(|f| f.is_some()).count();
    println!("{n} functions written; {mapped} of {} counters mapped to a file ({} functions in the profile)", files.len(), p.functions.len());
}
