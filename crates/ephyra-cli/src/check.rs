//! `ephyra check`: find the workspace's generators and strategies in a
//! static index of its source, and run each one's check test over
//! thousands of inputs, with reports, coverage pages and observations routed
//! into the artifact directory as for [`ephyra test`](crate::serve::run_tests).
//!
//! Predicates no generator serves, and generators naming a predicate that
//! does not resolve, are listed as work left.

use std::{
    collections::BTreeMap,
    io,
    process::{
        Command,
        Output,
        Stdio,
    },
};

use serde_json::Value;

use crate::{
    index::{
        Entry,
        Index,
        Kind,
    },
    serve::{
        self,
        Member,
        Target,
    },
};

/// What `ephyra check` runs, and how.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Artifact directory, target directory, instrumentation, cargo and its
    /// extra arguments, as for `ephyra test`.
    pub run: serve::Options,
    /// Only the checks of these functions; all when empty.
    pub names: Vec<String>,
    /// Print the index and the work left without building anything.
    pub list: bool,
    /// Inputs per check, overriding each check's own (`EPHYRA_TESTS`).
    pub tests: Option<u64>,
}

/// How a `ephyra check` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Every check passed and nothing is left to write.
    Clean,
    /// Every check passed, but a predicate lacks a generator or a generator
    /// names no predicate.
    WorkLeft,
    /// A check failed, did not build, or was not found.
    Failed,
}

impl Outcome {
    /// The process exit code: 0, 2 and 1.
    pub fn code(self) -> i32 {
        match self {
            Outcome::Clean => 0,
            Outcome::WorkLeft => 2,
            Outcome::Failed => 1,
        }
    }
}

/// Index the workspace in the current directory and run its checks.
pub fn run(options: &Options) -> io::Result<Outcome> {
    let members = serve::workspace_members(&options.run.cargo);
    if members.is_empty() {
        return Err(io::Error::other(
            "`cargo metadata` found no packages; run ephyra check inside a Cargo workspace",
        ));
    }
    let index = Index::of(&members);
    for warning in &index.warnings {
        println!("warning: {warning}");
    }
    let pairing = index.pairing();
    let work_left = !pairing.unserved.is_empty() || !pairing.unresolved.is_empty();
    list(&index);

    let mut failed = Vec::new();
    for name in &options.names {
        if !index.items.iter().any(|entry| entry.checked() && &entry.name == name) {
            failed.push(format!("{name}: no generator or strategy of that name"));
        }
    }
    let mut passed = Vec::new();
    if !options.list {
        let mut groups: BTreeMap<(String, String, String), Vec<&Entry>> = BTreeMap::new();
        for entry in index.items.iter().filter(|entry| entry.checked()) {
            if options.names.is_empty() || options.names.contains(&entry.name) {
                let key =
                    (entry.package.clone(), entry.target.kind.clone(), entry.target.name.clone());
                groups.entry(key).or_default().push(entry);
            }
        }
        for entries in groups.values() {
            run_group(options, &members, entries, &mut passed, &mut failed)?;
        }
    }

    println!();
    if !options.list {
        println!("ephyra check: {} passed, {} failed", passed.len(), failed.len());
        for name in &passed {
            println!("  passed  {name}");
        }
        for problem in &failed {
            println!("  FAILED  {problem}");
        }
    }
    for predicate in &pairing.unserved {
        let Kind::Predicate { input } = &predicate.kind else { continue };
        println!(
            "  needs a generator: {predicate}, a predicate over {input}; add \
             `#[ephyra::generator({})]` to a generator of values it accepts",
            predicate.name
        );
    }
    for (generator, problem) in &pairing.unresolved {
        println!("  unresolved: {generator} {problem}");
    }
    Ok(if !failed.is_empty() {
        Outcome::Failed
    } else if work_left {
        Outcome::WorkLeft
    } else {
        Outcome::Clean
    })
}

/// Print every indexed function.
fn list(index: &Index) {
    let count = index.items.len();
    println!("ephyra check: {count} annotated function{}", if count == 1 { "" } else { "s" });
    for entry in &index.items {
        let role = match entry.kind {
            Kind::Property { .. } => "property ",
            Kind::Predicate { .. } => "predicate",
            Kind::Generator { .. } => "generator",
        };
        println!("  {role}  {entry}: {}", entry.kind.describe());
    }
}

/// Build one target's test binary and run the checks of `entries` in it.
fn run_group(
    options: &Options,
    members: &[Member],
    entries: &[&Entry],
    passed: &mut Vec<String>,
    failed: &mut Vec<String>,
) -> io::Result<()> {
    let first = entries[0];
    let (cargo_args, notes) =
        serve::cargo_args_for(Some(&first.package), &options.run.cargo_args, members);
    for note in notes {
        println!("{note}");
    }
    let env = serve::run_env(&options.run);
    std::fs::create_dir_all(&options.run.dir)?;
    let mut build = vec!["test".to_string(), "-p".to_string(), first.package.clone()];
    build.extend(target_selector(&first.target));
    build.extend(cargo_args);
    build.extend(["--no-run", "--message-format=json"].map(String::from));
    println!("\n== building: {}", serve::command_line(&options.run.cargo, &build));
    let output = Command::new(&options.run.cargo)
        .args(&build)
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    let executable = serve::parse_artifacts(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .find(|artifact| {
            artifact.test && artifact.package == first.package && artifact.name == first.target.name
        })
        .map(|artifact| artifact.executable);
    let Some(executable) = executable.filter(|_| output.status.success()) else {
        println!("{}", compiler_output(&output));
        for entry in entries {
            failed.push(format!("{entry}: its test target did not build"));
        }
        return Ok(());
    };
    let listed = serve::list_tests(&executable)?;
    for entry in entries {
        let Some(test) = find_test(&listed, entry) else {
            failed.push(format!(
                "{entry}: no `{}` test in {}",
                entry.check_test(),
                executable.display()
            ));
            continue;
        };
        println!("\n== checking {entry}: {}", entry.kind.describe());
        let mut command = Command::new(&executable);
        command
            .args([test.as_str(), "--exact", "--ignored", "--nocapture"])
            .envs(env.iter().cloned())
            .current_dir(&entry.manifest_dir)
            .stdin(Stdio::null());
        if let Some(tests) = options.tests {
            command.env("EPHYRA_TESTS", tests.to_string());
        }
        if command.status()?.success() {
            passed.push(entry.to_string());
        } else {
            failed.push(entry.to_string());
        }
    }
    Ok(())
}

/// The cargo arguments that select `target` of a package.
fn target_selector(target: &Target) -> Vec<String> {
    match target.kind.as_str() {
        "test" => vec!["--test".to_string(), target.name.clone()],
        _ => vec!["--lib".to_string()],
    }
}

/// `entry`'s check test among a binary's tests: the one at its module path,
/// else the only one with its name.
fn find_test(listed: &[String], entry: &Entry) -> Option<String> {
    let name = entry.check_test();
    let at_module =
        entry.module.iter().cloned().chain([name.clone()]).collect::<Vec<_>>().join("::");
    if listed.contains(&at_module) {
        return Some(at_module);
    }
    let mut candidates =
        listed.iter().filter(|test| test.rsplit("::").next() == Some(name.as_str()));
    match (candidates.next(), candidates.next()) {
        (Some(only), None) => Some(only.clone()),
        _ => None,
    }
}

/// Cargo's stderr and the rendered compiler messages of a failed build.
fn compiler_output(output: &Output) -> String {
    let mut rendered = String::from_utf8_lossy(&output.stderr).into_owned();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        if value.get("reason").and_then(Value::as_str) == Some("compiler-message")
            && let Some(message) =
                value.get("message").and_then(|m| m.get("rendered")).and_then(Value::as_str)
        {
            rendered.push_str(message);
        }
    }
    rendered
}
