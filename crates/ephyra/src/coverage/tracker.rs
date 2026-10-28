// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Per-run coverage bookkeeping: which counters each input touched, what
//! was new, and the summary that goes into the report.

use std::{
    borrow::Cow,
    collections::BTreeSet,
    fmt,
    ops::Range,
    path::Path,
};

use crate::report::{
    Series,
    joined,
};

use super::counters::{
    Delta,
    Functions,
    Snapshot,
    snapshot_range,
};

/// AFL-style hit-count classes: a counter that went from "hit 3 times" to
/// "hit 30 times" in one input counts as new coverage once.
fn bucket(count: u64) -> u8 {
    match count {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4..=7 => 4,
        8..=15 => 5,
        16..=31 => 6,
        32..=127 => 7,
        _ => 8,
    }
}

/// Tracks coverage over the inputs of one run.
pub(crate) struct Tracker {
    /// Counter range covering the selected functions.
    window: Range<usize>,
    /// Indices into [`Functions::all`] of the selected functions.
    selected: Vec<usize>,
    /// Highest hit-count bucket seen per counter in the window.
    virgin: Vec<u8>,
    /// How many inputs touched each counter in the window.
    touched_by: Vec<u32>,
    /// Inputs observed.
    inputs: u64,
    /// Inputs that revealed new coverage.
    novel_inputs: u64,
    /// The input index (0-based, over observed inputs) at which coverage last grew.
    last_novelty_at: u64,
    /// Name prefix used to select functions (normalized), for the report.
    filter: String,
    /// Functions the filter itself selected, without the extras.
    matched: usize,
    /// Whether each window slot belongs to a selected function.
    selected_mask: Vec<bool>,
    /// Selected counters hit so far.
    hit_so_far: usize,
    /// (inputs observed, counters hit) whenever coverage grew.
    growth: Series,
}

/// What one input did.
pub(crate) struct Observation {
    /// The counters the input incremented.
    pub delta: Delta,
    /// How many counters, or hit-count classes, it reached for the first
    /// time in this run; nonzero means the input was novel.
    pub novelty: u32,
}

impl Tracker {
    /// Track the functions whose demangled names start with `filter`
    /// (`crate`, `crate::module` or `crate::Type`; see [`normalize_filter`]),
    /// or every function except ephyra's own when `filter` is empty, plus
    /// `extra` (indices into [`Functions::all`], for coverage targets).
    /// `None` when the binary is uninstrumented or nothing is selected.
    pub fn new(filter: &str, extra: &[usize]) -> Option<Tracker> {
        let filter = normalize_filter(filter);
        let functions = Functions::get();
        let mut selected: Vec<usize> =
            functions
                .all()
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    if filter.is_empty() { !is_own(&f.name) } else { matches(&f.name, &filter) }
                })
                .map(|(i, _)| i)
                .collect();
        let matched = selected.len();
        selected.extend(extra.iter().copied().filter(|i| *i < functions.all().len()));
        selected.sort_unstable();
        selected.dedup();
        if selected.is_empty() {
            return None;
        }
        let lo = selected.iter().map(|&i| functions.all()[i].counters.start).min()?;
        let hi = selected.iter().map(|&i| functions.all()[i].counters.end).max()?;
        let width = hi - lo;
        let mut selected_mask = vec![false; width];
        for &i in &selected {
            for c in functions.all()[i].counters.clone() {
                selected_mask[c - lo] = true;
            }
        }
        Some(Tracker {
            window: lo..hi,
            selected,
            virgin: vec![0; width],
            touched_by: vec![0; width],
            inputs: 0,
            novel_inputs: 0,
            last_novelty_at: 0,
            filter,
            matched,
            selected_mask,
            hit_so_far: 0,
            growth: Series::default(),
        })
    }

    /// Indices into [`Functions::all`] of the selected functions.
    pub fn selected(&self) -> &[usize] {
        &self.selected
    }

    /// The normalized filter.
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// How many functions the filter itself selected, not counting the
    /// coverage targets' and the property's own functions.
    pub fn matched(&self) -> usize {
        self.matched
    }

    /// Inputs observed so far.
    pub fn inputs(&self) -> u64 {
        self.inputs
    }

    /// Snapshot the window before running an input.
    pub fn begin(&self) -> Option<Snapshot> {
        snapshot_range(&self.window)
    }

    /// Record what the input did since `before`.
    pub fn end(&mut self, before: &Snapshot) -> Observation {
        let after = snapshot_range(&self.window).unwrap_or_else(|| before.clone());
        let delta = before.delta(&after);
        let mut novelty = 0u32;
        let mut grew = false;
        for &(index, count) in delta.hits() {
            let slot = index - self.window.start;
            // The window is one contiguous range, so it also holds counters
            // of whatever the linker placed between the selected functions;
            // those are bystanders and count for nothing, or novelty would
            // depend on the binary's layout.
            if !self.selected_mask[slot] {
                continue;
            }
            if self.touched_by[slot] == 0 {
                self.hit_so_far += 1;
                grew = true;
            }
            self.touched_by[slot] += 1;
            let class = bucket(count);
            if class > self.virgin[slot] {
                self.virgin[slot] = class;
                novelty += 1;
            }
        }
        if novelty > 0 {
            self.novel_inputs += 1;
            self.last_novelty_at = self.inputs;
        }
        self.inputs += 1;
        if grew {
            self.growth.push(self.inputs, self.hit_so_far as u64);
        }
        Observation { delta, novelty }
    }

    /// Summary for the report.
    pub fn summary(&self) -> Summary {
        let all = Functions::get().all();
        let mut reached = BTreeSet::new();
        let mut unreached = BTreeSet::new();
        let mut partial = Vec::new();
        let (mut counters_hit, mut counters_total) = (0, 0);
        for &i in &self.selected {
            let f = &all[i];
            let hit =
                f.counters.clone().filter(|c| self.touched_by[c - self.window.start] > 0).count();
            counters_hit += hit;
            counters_total += f.counters.len();
            if hit == 0 {
                unreached.insert(f.name.clone());
            } else {
                reached.insert(f.name.clone());
                if hit < f.counters.len() {
                    partial.push((f.name.clone(), hit, f.counters.len()));
                }
            }
        }
        partial.sort_by(|a, b| (b.2 - b.1).cmp(&(a.2 - a.1)).then_with(|| a.0.cmp(&b.0)));
        Summary {
            filter: self.filter.clone(),
            functions_reached: reached.len(),
            functions_total: self.selected.len(),
            counters_hit,
            counters_total,
            inputs: self.inputs,
            novel_inputs: self.novel_inputs,
            last_novelty_at: self.last_novelty_at,
            unreached: unreached.into_iter().collect(),
            partial,
            growth: self.growth.clone(),
            targets: Vec::new(),
        }
    }
}

/// Coverage of one run, as reported.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(serde::Serialize)]
pub struct Summary {
    /// The function-name prefix that was tracked (empty: everything but ephyra).
    pub filter: String,
    /// Selected functions reached by at least one input.
    pub functions_reached: usize,
    /// Selected functions in the binary.
    pub functions_total: usize,
    /// Counters (regions) hit by at least one input.
    pub counters_hit: usize,
    /// Counters (regions) in the selected functions.
    pub counters_total: usize,
    /// Inputs observed.
    pub inputs: u64,
    /// Inputs that revealed new coverage.
    pub novel_inputs: u64,
    /// The observed-input index at which coverage last grew.
    pub last_novelty_at: u64,
    /// Selected functions no input reached.
    pub unreached: Vec<String>,
    /// Reached functions with counters no input hit, as (name, hit, total),
    /// most missed first: the branches the generator does not exercise.
    pub partial: Vec<(String, usize, usize)>,
    /// Counters hit (y) after each number of inputs (x), recorded whenever
    /// coverage grew: a curve that flattens early means the generator has
    /// stopped reaching new code.
    pub growth: Series,
    /// The coverage targets and what happened to them.
    pub targets: Vec<super::targets::TargetOutcome>,
}

impl Summary {
    /// The coverage lines of the text report.
    pub(crate) fn write_report(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  coverage: {} of {} functions{} reached, {} of {} counters",
            self.functions_reached,
            self.functions_total,
            if self.filter.is_empty() { String::new() } else { format!(" in `{}`", self.filter) },
            self.counters_hit,
            self.counters_total
        )?;
        if !self.unreached.is_empty() {
            writeln!(f, "  never reached: {}", joined(self.unreached.iter().cloned(), 5))?;
        }
        for outcome in &self.targets {
            let what = if outcome.functions.is_empty() {
                "matches no instrumented code".to_string()
            } else if outcome.hits == 0 {
                format!(
                    "never reached; `{}` entered by {} inputs",
                    outcome.functions.join(", "),
                    outcome.function_inputs
                )
            } else {
                format!(
                    "hit by {} inputs (first at input {})",
                    outcome.hits,
                    outcome.first_hit_at.unwrap_or(0)
                )
            };
            writeln!(f, "  target {}: {what}", outcome.target)?;
        }
        if !self.partial.is_empty() {
            let partial =
                self.partial.iter().map(|(name, hit, total)| format!("{name} {hit}/{total}"));
            writeln!(f, "  partly covered: {}", joined(partial, 5))?;
        }
        Ok(())
    }
}

fn is_own(name: &str) -> bool {
    let bare = name.trim_start_matches('<');
    bare.starts_with("ephyra::") || bare.starts_with("ephyra_macros::")
}

/// The filter as matched: surrounding whitespace, a leading `<` and a
/// trailing `>` trimmed, and `>::` read as `::`, so the way a method's name
/// demangles (`<my_crate::Type>::method`) and partial copies of it
/// (`<my_crate::Type`, `<my_crate::Type>`) mean `my_crate::Type::method` and
/// `my_crate::Type`.
pub(crate) fn normalize_filter(filter: &str) -> String {
    let filter = filter.trim().trim_start_matches('<').trim_end_matches('>');
    filter.replace(">::", "::")
}

/// A demangled name as matched: leading `<`s trimmed and `>::` read as
/// `::`, the way [`normalize_filter`] reads a filter, so
/// `<my_crate::Type>::method` is `my_crate::Type::method`.
fn normalize_name(name: &str) -> Cow<'_, str> {
    let bare = name.trim_start_matches('<');
    if bare.contains(">::") { Cow::Owned(bare.replace(">::", "::")) } else { Cow::Borrowed(bare) }
}

/// Whether `name` is `prefix` or lies under it: the prefix must end at a
/// path boundary (`::`), at the type's generic arguments (`<`), or before
/// ` as Trait` in a trait method's name, so `my_crate` never matches
/// `my_crate_other::f`.
fn matches(name: &str, prefix: &str) -> bool {
    let name = normalize_name(name);
    let Some(rest) = name.strip_prefix(prefix) else { return false };
    rest.is_empty() || rest.starts_with("::") || rest.starts_with('<') || rest.starts_with(" as ")
}

/// A crate-level filter selecting fewer functions than this is almost
/// surely the wrong crate (or the test crate itself): real code under test
/// has more. Module and type filters are narrow by choice and exempt.
const FEW_FUNCTIONS: usize = 5;

/// The `coverage_filter` health warning for a filter that selected
/// `matched` functions, if it deserves one. `own_crate`: the filter is the
/// crate the property is defined in, as the defaults choose.
pub(crate) fn filter_warning(filter: &str, matched: usize, own_crate: bool) -> Option<String> {
    let what = if matched == 0 {
        "matches no instrumented functions".to_string()
    } else if !filter.is_empty() && !filter.contains("::") && matched < FEW_FUNCTIONS {
        format!("selects only {matched} {}", if matched == 1 { "function" } else { "functions" })
    } else {
        return None;
    };
    let own = if own_crate {
        ", and it is the crate the property is defined in, so a separate test crate covers only itself"
    } else {
        ""
    };
    Some(format!(
        "the coverage filter `{filter}` {what}{own}; point it at the code under test with \
         EPHYRA_COVERAGE_FILTER=<crate> or `config.coverage.filter = \"<crate>\"`, as `crate` or \
         `crate::module` (the default is the test's own package)"
    ))
}

/// Write a profraw image of the process's counters to `<dir>/<name>.profraw`
/// for `llvm-cov`. The image covers everything that ran in the process so
/// far, not only this property.
pub(crate) fn write_profraw(dir: &Path, name: &str) -> std::io::Result<std::path::PathBuf> {
    let image = super::counters::profraw_image()
        .ok_or_else(|| std::io::Error::other("coverage unavailable"))?;
    crate::write_named(dir, name, "profraw", &image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_monotone_classes() {
        let classes: Vec<u8> = [0u64, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 127, 128, 10_000]
            .iter()
            .map(|&c| bucket(c))
            .collect();
        assert_eq!(classes, vec![0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8]);
    }

    #[test]
    fn name_matching() {
        assert!(matches("my_crate::a::b", "my_crate"));
        assert!(matches("<my_crate::T as Trait>::f", "my_crate"));
        assert!(!matches("my_crate_other::a", "my_crate"));
        assert!(is_own("ephyra::runner::check"));
        assert!(!is_own("crabby::x"));
    }

    /// Match `name` against `filter` as the tracker does.
    fn selects(filter: &str, name: &str) -> bool {
        matches(name, &normalize_filter(filter))
    }

    #[test]
    fn a_crate_filter_selects_free_functions_and_methods() {
        for name in [
            "sqlparser::parse",
            "sqlparser::parser::Parser::new",
            "<sqlparser::parser::Parser>::parse_select",
            "<sqlparser::parser::Parser as core::fmt::Debug>::fmt",
            "<sqlparser::ast::Expr<u8>>::span",
        ] {
            assert!(selects("sqlparser", name), "{name}");
            assert!(selects(" sqlparser\n", name), "{name}");
        }
    }

    #[test]
    fn a_crate_filter_has_no_false_prefix_matches() {
        assert!(!selects("sqlparser", "sqlparser_ephyra::props::roundtrip"));
        assert!(!selects("sqlparser", "<sqlparser_ephyra::Gen>::generate"));
        assert!(!selects("sqlparser::parser", "sqlparser::parser_utils::f"));
        assert!(!selects("sqlparser::parser::Parser", "<sqlparser::parser::ParserError>::fmt"));
        assert!(!selects("sqlparser", "<core::option::Option<sqlparser::ast::Expr>>::map"));
    }

    #[test]
    fn module_and_type_filters_in_any_spelling() {
        let method = "<sqlparser::parser::Parser>::parse_select";
        let trait_method = "<sqlparser::parser::Parser as core::fmt::Debug>::fmt";
        for filter in [
            "sqlparser::parser",
            "sqlparser::parser::Parser",
            "<sqlparser::parser::Parser>",
            "<sqlparser::parser::Parser",
            "sqlparser::parser::Parser>",
        ] {
            assert!(selects(filter, method), "{filter}");
            assert!(selects(filter, trait_method), "{filter}");
            assert!(!selects(filter, "sqlparser::tokenizer::Tokenizer::new"), "{filter}");
        }
        for filter in [
            "<sqlparser::parser::Parser>::parse_select",
            "sqlparser::parser::Parser>::parse_select",
            "sqlparser::parser::Parser::parse_select",
        ] {
            assert!(selects(filter, method), "{filter}");
            assert!(selects(filter, "sqlparser::parser::Parser::parse_select"), "{filter}");
            assert!(!selects(filter, "<sqlparser::parser::Parser>::parse_selection"), "{filter}");
        }
        assert_eq!(normalize_filter("<a::B>::c"), "a::B::c");
        assert_eq!(normalize_filter("  a::b "), "a::b");
    }

    #[test]
    fn filter_warnings() {
        let none = filter_warning("sqlparser", 0, false).expect("a warning");
        assert!(none.contains("`sqlparser` matches no instrumented functions"), "{none}");
        assert!(none.contains("EPHYRA_COVERAGE_FILTER=<crate>"), "{none}");
        assert!(none.contains("`crate` or `crate::module`"), "{none}");
        assert!(!none.contains("defined in"), "{none}");
        let few = filter_warning("my_tests", 3, true).expect("a warning");
        assert!(few.contains("selects only 3 functions"), "{few}");
        assert!(few.contains("the crate the property is defined in"), "{few}");
        assert!(filter_warning("my_tests", 1, false).unwrap().contains("only 1 function;"));
        // Enough functions, or a deliberately narrow module or type.
        assert_eq!(filter_warning("my_lib", 40, true), None);
        assert_eq!(filter_warning("my_lib::parser", 2, false), None);
        assert_eq!(filter_warning("", 3, false), None);
        assert!(filter_warning("my_lib::parser", 0, false).is_some());
    }

    /// Under instrumentation the normalized spellings select this very
    /// module's functions, methods included.
    #[cfg(ephyra_instrumented)]
    #[test]
    fn normalized_filters_select_instrumented_functions() {
        let names = |filter: &str| -> Vec<String> {
            let tracker = Tracker::new(filter, &[]).expect("functions in this module");
            assert_eq!(tracker.matched(), tracker.selected().len());
            let all = Functions::get().all();
            tracker.selected().iter().map(|&i| all[i].name.clone()).collect()
        };
        let plain = names("ephyra::coverage::tracker::Tracker");
        assert!(plain.iter().any(|n| n.contains("Tracker>::summary")), "{plain:?}");
        assert_eq!(names("<ephyra::coverage::tracker::Tracker>"), plain);
        assert_eq!(names("<ephyra::coverage::tracker::Tracker"), plain);
        assert_eq!(names(" ephyra::coverage::tracker::Tracker "), plain);
        let method = names("<ephyra::coverage::tracker::Tracker>::summary");
        assert!(!method.is_empty(), "{method:?}");
        assert!(method.iter().all(|n| n.contains("Tracker>::summary")), "{method:?}");
        assert!(Tracker::new("ephyra::coverage::tracker", &[]).unwrap().matched() > plain.len());
        assert!(Tracker::new("ephyra_no_such_crate", &[]).is_none());
    }

    #[test]
    fn tracker_matches_the_build() {
        match Tracker::new("", &[]) {
            None => assert!(!super::super::available()),
            Some(mut tracker) => {
                let before = tracker.begin().unwrap();
                let observation = tracker.end(&before);
                assert_eq!(tracker.inputs, 1);
                let _ = observation;
                let summary = tracker.summary();
                assert_eq!(summary.functions_total, tracker.selected.len());
            },
        }
    }
}
