// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Coverage targets, described under "Targets" in the [module docs](super).

use std::collections::{
    BTreeMap,
    BTreeSet,
};

use crate::config::CoverageTarget;

use super::{
    counters::{
        Delta,
        Functions,
    },
    locate::same_file,
    mapping::{
        Kind,
        Mapping,
    },
};

/// What happened to one coverage target over the run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(serde::Serialize)]
pub struct TargetOutcome {
    /// The target as configured.
    pub target: CoverageTarget,
    /// Functions whose code lies in the target; empty when it matched no
    /// instrumented code.
    pub functions: Vec<String>,
    /// Inputs that executed a line inside the target.
    pub hits: u64,
    /// The observed-input index at which the target was first hit.
    pub first_hit_at: Option<u64>,
    /// Inputs that entered any of the target's functions.
    pub function_inputs: u64,
}

struct State {
    outcome: TargetOutcome,
    /// (function index, file index, line) triples inside the target whose
    /// innermost region belongs to that function.
    lines: Vec<(usize, usize, u32)>,
}

/// The configured targets, resolved against the mapping.
pub(crate) struct Targets {
    mapping: &'static Mapping,
    states: Vec<State>,
    /// Function index to the targets it belongs to.
    by_function: BTreeMap<usize, Vec<usize>>,
}

impl Targets {
    /// Resolve targets to lines and functions; `None` without a mapping.
    pub fn resolve(targets: Vec<CoverageTarget>) -> Option<Targets> {
        let mapping = Mapping::get()?;
        let all = Functions::get().all();
        let mut states = Vec::with_capacity(targets.len());
        let mut by_function: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for target in targets {
            let mut state = State {
                outcome: TargetOutcome {
                    target,
                    functions: Vec::new(),
                    hits: 0,
                    first_hit_at: None,
                    function_inputs: 0,
                },
                lines: Vec::new(),
            };
            let target = &state.outcome.target;
            let files: Vec<usize> = mapping
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| same_file(f, &target.file))
                .map(|(i, _)| i)
                .collect();
            if files.is_empty() {
                states.push(state);
                continue;
            }
            let range = target.begin_line..=target.end_line;
            for (fi, function) in all.iter().enumerate() {
                let Some(mapped) = mapping.function(function.name_ref, function.func_hash) else {
                    continue;
                };
                let overlaps = mapped.regions.iter().any(|region| {
                    matches!(region.kind, Kind::Code | Kind::Branch)
                        && files.contains(&region.file)
                        && region.start_line <= *range.end()
                        && *range.start() <= region.end_line
                });
                if !overlaps {
                    continue;
                }
                // The lines of the target this function owns; a nested closure
                // is a separate function with its own regions.
                let mut any = false;
                for ((file, line), _) in mapped.line_counts(&[]) {
                    if files.contains(&file) && range.contains(&line) {
                        state.lines.push((fi, file, line));
                        any = true;
                    }
                }
                if any {
                    state.outcome.functions.push(function.name.clone());
                    by_function.entry(fi).or_default().push(states.len());
                }
            }
            states.push(state);
        }
        Some(Targets { mapping, states, by_function })
    }

    /// The functions that own target lines, for the tracked set.
    pub fn functions(&self) -> Vec<usize> {
        self.by_function.keys().copied().collect()
    }

    /// The targets that matched no instrumented code.
    pub fn unresolved(&self) -> impl Iterator<Item = &CoverageTarget> + '_ {
        self.states.iter().filter(|s| s.outcome.functions.is_empty()).map(|s| &s.outcome.target)
    }

    /// Note an input; returns whether it hit a target line.
    pub fn observe(&mut self, delta: &Delta, input: u64) -> bool {
        let all = Functions::get().all();
        let mut entered: BTreeSet<usize> = BTreeSet::new();
        let mut hit: BTreeSet<usize> = BTreeSet::new();
        let mut values = Vec::new();
        for (&fi, owners) in &self.by_function {
            if !delta.fill(&all[fi].counters, &mut values) {
                continue;
            }
            let Some(mapped) = self.mapping.function(all[fi].name_ref, all[fi].func_hash) else {
                continue;
            };
            let counts = mapped.line_counts(&values);
            for &si in owners {
                entered.insert(si);
                let reached = self.states[si].lines.iter().any(|&(f, file, line)| {
                    f == fi && counts.get(&(file, line)).is_some_and(|c| *c > 0)
                });
                if reached {
                    hit.insert(si);
                }
            }
        }
        for si in entered {
            self.states[si].outcome.function_inputs += 1;
        }
        for &si in &hit {
            let outcome = &mut self.states[si].outcome;
            outcome.hits += 1;
            outcome.first_hit_at.get_or_insert(input);
        }
        !hit.is_empty()
    }

    /// The outcome of every target.
    pub fn outcomes(self) -> Vec<TargetOutcome> {
        self.states.into_iter().map(|s| s.outcome).collect()
    }
}
