// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `ephyra` command: run tests into one artifact directory, check
//! generators and strategies, then explore and rerun them from the local
//! visualization server.

use std::path::PathBuf;

use ephyra_cli::{
    check,
    serve,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("test") => test(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("guide") => print!("{}", ephyra_cli::GUIDE),
        Some("visualize") => visualize(&args[1..]),
        None | Some("--help" | "-h" | "help") => usage(0),
        Some(other) => {
            eprintln!("unknown command `{other}`");
            usage(2);
        },
    }
}

fn usage(code: i32) -> ! {
    eprintln!(
        "usage:\n\
           ephyra test [--dir DIR] [--no-instrument] [--target-dir DIR] [--cargo PATH] [--] [CARGO TEST ARGS...]\n\
           ephyra check [NAME...] [--list] [--tests N] [--dir DIR] [--no-instrument] [--target-dir DIR] [--cargo PATH] [-- CARGO ARGS...]\n\
           ephyra guide\n\
           ephyra visualize [--dir DIR] [--port PORT] [--cargo-args ARGS] [--no-instrument] [--target-dir DIR] [--cargo PATH]\n\n\
         `test` runs cargo test with Ephyra reports, traces, coverage pages, targets and\n\
         regressions routed into DIR (default .ephyra). Instrumentation is on by default.\n\n\
         `check` reads the workspace's #[ephyra::test] and #[ephyra::generator]\n\
         functions from source and runs the check of every generator and strategy (or of\n\
         the NAMEd ones) over thousands of inputs, into the same DIR. It lists predicates\n\
         that still need a generator, and exits 1 if a check failed, 2 if only work is left.\n\n\
         `guide` prints how to write generators and strategies that pass `check`.\n\n\
         `visualize` serves those artifacts on 127.0.0.1:PORT (default 7878) and can\n\
         discover or rerun tests.\n\n\
         --list              print what `check` found, without building\n\
         --tests N           inputs per check (default 10000)\n\
         --cargo-args ARGS   extra arguments for every cargo run from the visualization\n\
         --no-instrument     do not append -C instrument-coverage to RUSTFLAGS\n\
         --target-dir DIR    CARGO_TARGET_DIR (default: environment, else target/cov)\n\
         --cargo PATH        cargo executable (default: cargo)"
    );
    std::process::exit(code);
}

fn next_value(args: &[String], i: usize, flag: &str) -> String {
    match args.get(i + 1) {
        Some(value) => value.clone(),
        None => {
            eprintln!("{flag} needs a value");
            usage(2);
        },
    }
}

/// Parse an option `test` and `visualize` share into `options` and step `i`
/// past it; false when `args[*i]` is not one.
fn common_option(options: &mut serve::Options, args: &[String], i: &mut usize) -> bool {
    match args[*i].as_str() {
        "--dir" => options.dir = PathBuf::from(next_value(args, *i, "--dir")),
        "--target-dir" => {
            options.target_dir = Some(PathBuf::from(next_value(args, *i, "--target-dir")));
        },
        "--cargo" => options.cargo = PathBuf::from(next_value(args, *i, "--cargo")),
        "--no-instrument" => {
            options.instrument = false;
            *i += 1;
            return true;
        },
        "--help" | "-h" => usage(0),
        _ => return false,
    }
    *i += 2;
    true
}

fn test(args: &[String]) {
    let mut options = serve::Options::default();
    let mut i = 0;
    while i < args.len() {
        if common_option(&mut options, args, &mut i) {
            continue;
        }
        options.cargo_args.extend(args[i..].iter().cloned());
        break;
    }
    let args = [vec!["test".to_string()], options.cargo_args.clone()].concat();
    println!("ephyra test: {}", serve::command_line(&options.cargo, &args));
    println!("  artifacts {}", options.dir.display());
    match serve::run_tests(&options) {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("ephyra test: could not run cargo: {e}");
            std::process::exit(1);
        },
    }
}

fn check(args: &[String]) {
    let mut options = check::Options::default();
    let mut i = 0;
    while i < args.len() {
        if common_option(&mut options.run, args, &mut i) {
            continue;
        }
        match args[i].as_str() {
            "--list" => options.list = true,
            "--tests" => {
                options.tests = match next_value(args, i, "--tests").parse() {
                    Ok(tests) => Some(tests),
                    Err(_) => {
                        eprintln!("--tests needs a number");
                        std::process::exit(2);
                    },
                };
                i += 1;
            },
            "--" => {
                options.run.cargo_args.extend(args[i + 1..].iter().cloned());
                break;
            },
            name => options.names.push(name.to_string()),
        }
        i += 1;
    }
    match check::run(&options) {
        Ok(outcome) => std::process::exit(outcome.code()),
        Err(e) => {
            eprintln!("ephyra check: {e}");
            std::process::exit(1);
        },
    }
}

fn visualize(args: &[String]) {
    let mut options = serve::Options::default();
    let mut i = 0;
    while i < args.len() {
        if common_option(&mut options, args, &mut i) {
            continue;
        }
        match args[i].as_str() {
            "--port" => {
                options.port = match next_value(args, i, "--port").parse() {
                    Ok(port) => port,
                    Err(_) => {
                        eprintln!("--port needs a number");
                        std::process::exit(2);
                    },
                };
                i += 2;
            },
            "--cargo-args" => {
                options.cargo_args.extend(
                    next_value(args, i, "--cargo-args").split_whitespace().map(String::from),
                );
                i += 2;
            },
            other => {
                eprintln!("unknown argument `{other}`");
                usage(2);
            },
        }
    }
    let dir = options.dir.clone();
    let cargo_args = options.cargo_args.join(" ");
    let instrument = options.instrument;
    let server = match serve::start(options) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("ephyra visualize: could not listen: {e}");
            std::process::exit(1);
        },
    };
    println!("ephyra visualize: {}", server.url());
    println!(
        "  pages   {}/coverage/*.html   (EPHYRA_COVERAGE_HTML={}/coverage)",
        dir.display(),
        dir.display()
    );
    println!(
        "  reports {}/reports.jsonl     (EPHYRA_REPORT_JSON={}/reports.jsonl)",
        dir.display(),
        dir.display()
    );
    println!("  traces  {}/observations/*.jsonl", dir.display());
    println!("  targets {}/targets.jsonl     (read by the next run)", dir.display());
    println!("  tests   {}/tests.json        (written by \"Discover tests\")", dir.display());
    println!(
        "  runs    cargo {}{} from {}",
        if cargo_args.is_empty() { "…".to_string() } else { format!("… {cargo_args}") },
        if instrument { " with -C instrument-coverage" } else { "" },
        std::env::current_dir().map_or("here".to_string(), |d| d.display().to_string())
    );
    println!("Press Ctrl-C to stop.");
    loop {
        std::thread::park();
    }
}
