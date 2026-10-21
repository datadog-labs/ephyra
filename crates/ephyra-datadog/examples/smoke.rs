//! Send a passing and a failing run to the local Datadog agent, tagged
//! `run:<name>`, with sampled inputs logged to `<name>.jsonl`:
//!
//! ```text
//! cargo run -p ephyra-datadog --example smoke -- <name>
//! ```

use ephyra::{
    Config,
    TestResult,
    check,
    gens,
};

fn main() {
    let run = std::env::args().nth(1).unwrap_or_else(|| "smoke".to_string());
    ephyra_datadog::Datadog::new()
        .tag(format!("run:{run}"))
        .log_file(format!("{run}.jsonl"), 100)
        .install();
    let config = Config::default().tests(5_000).no_regressions();
    let sorted = check(
        &config.clone().name("smoke-sorted"),
        gens::vec(gens::int(0i32..=100)),
        |mut v: Vec<i32>| {
            ephyra::feature("len", v.len());
            ephyra::feature("parity", if v.len().is_multiple_of(2) { "even" } else { "odd" });
            ephyra::cover(v.len() > 10, "long");
            ephyra::observe("sum", || v.iter().sum::<i32>());
            v.sort();
            v.windows(2).all(|w| w[0] <= w[1])
        },
    );
    let small = check(&config.name("smoke-small"), gens::int(0i32..=1000), |n: i32| {
        ephyra::feature("n", n);
        TestResult::check(n < 990, "too big")
    });
    println!("{sorted}\n{small}");
}
