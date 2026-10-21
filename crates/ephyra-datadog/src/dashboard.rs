//! A Datadog dashboard for what [`Datadog`](crate::Datadog) sends, laid out
//! from the properties and features that reached Datadog.
//!
//! [`discovery_queries`] lists the metric queries whose series name each
//! property's features, labels, covers and observations;
//! [`Schema::add_series`] reads their results back, and [`dashboard`] turns
//! the schema into the dashboard's JSON (`POST /api/v1/dashboard`). The
//! `ephyra-datadog` binary (feature `cli`) does all three against the API.
//!
//! The dashboard starts with an overview of every property (inputs by status
//! and origin, runs by result, timing), then has one
//! group per property: a chart per numeric feature and observation, the
//! values of each text feature, label counts, the share of inputs meeting
//! each `cover`, target scores, timing by origin and line coverage. Template
//! variables pick the property, run, env and version.

use std::collections::{
    BTreeMap,
    BTreeSet,
};

use serde_json::{
    Value,
    json,
};

/// What one property recorded, by kind; names as Datadog stores them (see
/// the crate docs on tags).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertySchema {
    /// Numeric features (`feature` tag of `<prefix>.feature`).
    pub numeric_features: BTreeSet<String>,
    /// Text features (`feature` tag of `<prefix>.feature.value`).
    pub text_features: BTreeSet<String>,
    /// Labels.
    pub labels: BTreeSet<String>,
    /// Cover conditions.
    pub covers: BTreeSet<String>,
    /// Numeric observations.
    pub observations: BTreeSet<String>,
    /// Whether it reported `target` scores.
    pub target: bool,
    /// Whether it sent line coverage (an instrumented build).
    pub coverage: bool,
}

/// The properties a dashboard shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    /// By property name.
    pub properties: BTreeMap<String, PropertySchema>,
}

/// What a discovery query's series name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovered {
    /// Properties that ran.
    Properties,
    /// Numeric features.
    NumericFeatures,
    /// Text features.
    TextFeatures,
    /// Labels.
    Labels,
    /// Cover conditions.
    Covers,
    /// Numeric observations.
    Observations,
    /// Properties that reported target scores.
    Targets,
    /// Properties that sent line coverage.
    Coverage,
}

impl Discovered {
    /// The tag a series of this kind names besides `property`.
    fn tag(self) -> Option<&'static str> {
        match self {
            Discovered::Properties | Discovered::Targets | Discovered::Coverage => None,
            Discovered::NumericFeatures | Discovered::TextFeatures => Some("feature"),
            Discovered::Labels => Some("label"),
            Discovered::Covers => Some("cover"),
            Discovered::Observations => Some("observation"),
        }
    }
}

/// The metric queries whose series (grouped by property and the kind's
/// tag) make up a [`Schema`], for metrics named `<prefix>.*` and tags
/// matching `scope` (such as `env:ci`, or `*`).
pub fn discovery_queries(prefix: &str, scope: &str) -> Vec<(Discovered, String)> {
    vec![
        (
            Discovered::Properties,
            format!("sum:{prefix}.cases{{{scope}}} by {{property}}.as_count()"),
        ),
        (
            Discovered::NumericFeatures,
            format!("count:{prefix}.feature{{{scope}}} by {{property,feature}}"),
        ),
        (
            Discovered::TextFeatures,
            format!("sum:{prefix}.feature.value{{{scope}}} by {{property,feature}}.as_count()"),
        ),
        (
            Discovered::Labels,
            format!("sum:{prefix}.label{{{scope}}} by {{property,label}}.as_count()"),
        ),
        (
            Discovered::Covers,
            format!("sum:{prefix}.cover{{{scope}}} by {{property,cover}}.as_count()"),
        ),
        (
            Discovered::Observations,
            format!("count:{prefix}.observation{{{scope}}} by {{property,observation}}"),
        ),
        (Discovered::Targets, format!("count:{prefix}.target{{{scope}}} by {{property}}")),
        (
            Discovered::Coverage,
            format!("count:{prefix}.coverage.case_lines{{{scope}}} by {{property}}"),
        ),
    ]
}

impl Schema {
    /// Add what a series of a discovery query of kind `kind` names; `scope`
    /// is the series' tag set as the query API returns it
    /// (`property:p,feature:f`).
    pub fn add_series(&mut self, kind: Discovered, scope: &str) {
        let tags: BTreeMap<&str, &str> =
            scope.split(',').filter_map(|t| t.split_once(':')).collect();
        let Some(property) = tags.get("property") else { return };
        let schema = self.properties.entry(property.to_string()).or_default();
        match kind {
            Discovered::Targets => schema.target = true,
            Discovered::Coverage => schema.coverage = true,
            _ => {},
        }
        let Some(name) = kind.tag().and_then(|tag| tags.get(tag)) else { return };
        let set = match kind {
            Discovered::Properties | Discovered::Targets | Discovered::Coverage => return,
            Discovered::NumericFeatures => &mut schema.numeric_features,
            Discovered::TextFeatures => &mut schema.text_features,
            Discovered::Labels => &mut schema.labels,
            Discovered::Covers => &mut schema.covers,
            Discovered::Observations => &mut schema.observations,
        };
        set.insert(name.to_string());
    }
}

/// The template variables every query is filtered by.
const VARIABLES: &str = "$run,$env,$version";

/// The dashboard JSON for `schema`, titled `title`, over metrics named
/// `<prefix>.*`.
pub fn dashboard(title: &str, prefix: &str, schema: &Schema) -> Value {
    let all = format!("$property,{VARIABLES}");
    let mut widgets = vec![overview(prefix, &all)];
    for (property, recorded) in &schema.properties {
        widgets.push(property_group(prefix, property, recorded));
    }
    let variables: Vec<Value> = ["property", "run", "env", "version"]
        .iter()
        .map(|name| json!({"name": name, "prefix": name, "default": "*"}))
        .collect();
    json!({
        "title": title,
        "description": "Generated by ephyra-datadog from the properties and features that reached Datadog; regenerate it after adding some. Metrics come from the ephyra_datadog::Datadog sink.",
        "layout_type": "ordered",
        "template_variables": variables,
        "widgets": widgets,
    })
}

fn overview(p: &str, f: &str) -> Value {
    group(
        "Overview",
        vec![
            note(
                "Inputs by status (`passed`, `failed`, `gave_up`) and origin (`random`, `explore`, `mutation`, `shrink`, `neighbourhood`, `minimal`, ...), finished runs by result, and per-input timing. Runs that do not pass also send an event with their report (search events for `ephyra`).",
            ),
            query_value("Inputs", &format!("sum:{p}.cases{{{f}}}.as_count()"), "sum"),
            query_value(
                "Failed runs",
                &format!("sum:{p}.runs{{{f},result:failed}}.as_count()"),
                "sum",
            ),
            query_value(
                "Passed runs",
                &format!("sum:{p}.runs{{{f},result:passed}}.as_count()"),
                "sum",
            ),
            ts(
                "Inputs, by status",
                &[format!("sum:{p}.cases{{{f}}} by {{status}}.as_count()")],
                "bars",
            ),
            ts(
                "Inputs, by origin",
                &[format!("sum:{p}.cases{{{f}}} by {{origin}}.as_count()")],
                "bars",
            ),
            toplist(
                "Runs, by property and result",
                &format!("sum:{p}.runs{{{f}}} by {{property,result}}.as_count()"),
            ),
            ts(
                "Seconds per input (avg, p90), by property",
                &[
                    format!("avg:{p}.time.execute{{{f},origin:random}} by {{property}}"),
                    format!("p90:{p}.time.execute{{{f},origin:random}} by {{property}}"),
                ],
                "line",
            ),
        ],
    )
}

fn property_group(p: &str, property: &str, recorded: &PropertySchema) -> Value {
    let f = format!("property:{property},{VARIABLES}");
    let mut widgets = vec![
        ts(
            "Inputs, by status",
            &[format!("sum:{p}.cases{{{f}}} by {{status}}.as_count()")],
            "bars",
        ),
        ts(
            "Inputs, by origin",
            &[format!("sum:{p}.cases{{{f}}} by {{origin}}.as_count()")],
            "bars",
        ),
    ];
    for feature in &recorded.numeric_features {
        widgets.push(spread(
            &format!("Feature {feature}"),
            p,
            "feature",
            &format!("{f},feature:{feature}"),
        ));
    }
    for feature in &recorded.text_features {
        widgets.push(toplist(
            &format!("Feature {feature}, by value"),
            &format!("sum:{p}.feature.value{{{f},feature:{feature},status:passed}} by {{value}}.as_count()"),
        ));
    }
    if !recorded.labels.is_empty() {
        widgets.push(toplist(
            "Labels",
            &format!("sum:{p}.label{{{f},status:passed}} by {{label}}.as_count()"),
        ));
    }
    for cover in &recorded.covers {
        let c = format!("{f},cover:{cover},status:passed");
        widgets.push(formula_ts(
            &format!("Cover {cover}: % of passing inputs"),
            &[
                ("met", format!("sum:{p}.cover{{{c},met:true}}.as_count()")),
                ("unmet", format!("sum:{p}.cover{{{c},met:false}}.as_count()")),
            ],
            "100 * met / (met + unmet)",
            "% met",
        ));
    }
    for observation in &recorded.observations {
        widgets.push(spread(
            &format!("Observation {observation}"),
            p,
            "observation",
            &format!("{f},observation:{observation}"),
        ));
    }
    if recorded.target {
        widgets.push(ts(
            "Target score (avg, max)",
            &[format!("avg:{p}.target{{{f}}}"), format!("max:{p}.target{{{f}}}")],
            "line",
        ));
    }
    widgets.push(ts(
        "Seconds per input (avg, p90), by origin",
        &[
            format!("avg:{p}.time.execute{{{f}}} by {{origin}}"),
            format!("p90:{p}.time.execute{{{f}}} by {{origin}}"),
            format!("avg:{p}.time.generate{{{f}}} by {{origin}}"),
        ],
        "line",
    ));
    if recorded.coverage {
        widgets.push(ts(
            "Lines covered (run total; per input avg)",
            &[
                format!("max:{p}.coverage.lines{{{f}}} by {{run}}"),
                format!("avg:{p}.coverage.case_lines{{{f}}}"),
            ],
            "line",
        ));
    }
    group(&format!("Property {property}"), widgets)
}

/// A distribution metric's average, median, 90th percentile and maximum
/// over passing inputs.
fn spread(title: &str, p: &str, metric: &str, f: &str) -> Value {
    let q = |agg: &str| format!("{agg}:{p}.{metric}{{{f},status:passed}}");
    ts(&format!("{title} (avg, p50, p90, max)"), &[q("avg"), q("p50"), q("p90"), q("max")], "line")
}

fn ts(title: &str, queries: &[String], display: &str) -> Value {
    json!({"definition": {
        "type": "timeseries",
        "title": title,
        "show_legend": true,
        "requests": queries.iter().map(|q| json!({"q": q, "display_type": display})).collect::<Vec<_>>(),
    }})
}

fn formula_ts(title: &str, queries: &[(&str, String)], formula: &str, alias: &str) -> Value {
    json!({"definition": {
        "type": "timeseries",
        "title": title,
        "show_legend": true,
        "requests": [{
            "response_format": "timeseries",
            "display_type": "line",
            "queries": queries
                .iter()
                .map(|(name, q)| json!({"data_source": "metrics", "name": name, "query": q}))
                .collect::<Vec<_>>(),
            "formulas": [{"formula": formula, "alias": alias}],
        }],
    }})
}

fn query_value(title: &str, q: &str, aggregator: &str) -> Value {
    json!({"definition": {
        "type": "query_value",
        "title": title,
        "precision": 0,
        "requests": [{"q": q, "aggregator": aggregator}],
    }})
}

fn toplist(title: &str, q: &str) -> Value {
    json!({"definition": {"type": "toplist", "title": title, "requests": [{"q": q}]}})
}

fn note(text: &str) -> Value {
    json!({"definition": {
        "type": "note",
        "content": text,
        "background_color": "white",
        "font_size": "14",
        "text_align": "left",
        "show_tick": false,
        "has_padding": true,
    }})
}

fn group(title: &str, widgets: Vec<Value>) -> Value {
    json!({"definition": {"type": "group", "title": title, "layout_type": "ordered", "widgets": widgets}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        let mut schema = Schema::default();
        for (kind, scope) in [
            (Discovered::Properties, "property:smoke-sorted,run:r1"),
            (Discovered::Properties, "property:smoke-small"),
            (Discovered::NumericFeatures, "feature:len,property:smoke-sorted"),
            (Discovered::NumericFeatures, "feature:n,property:smoke-small"),
            (Discovered::TextFeatures, "feature:parity,property:smoke-sorted"),
            (Discovered::Covers, "cover:long,property:smoke-sorted"),
            (Discovered::Observations, "observation:sum,property:smoke-sorted"),
            (Discovered::Labels, "label:seen,property:smoke-small"),
            (Discovered::Labels, "label:orphan"),
            (Discovered::Targets, "property:smoke-small"),
        ] {
            schema.add_series(kind, scope);
        }
        schema
    }

    #[test]
    fn series_scopes_build_the_schema() {
        let schema = schema();
        assert_eq!(schema.properties.keys().collect::<Vec<_>>(), ["smoke-small", "smoke-sorted"]);
        let sorted = &schema.properties["smoke-sorted"];
        assert_eq!(sorted.numeric_features, BTreeSet::from(["len".to_string()]));
        assert_eq!(sorted.text_features, BTreeSet::from(["parity".to_string()]));
        assert_eq!(sorted.covers, BTreeSet::from(["long".to_string()]));
        assert_eq!(sorted.observations, BTreeSet::from(["sum".to_string()]));
        assert!(sorted.labels.is_empty());
        assert_eq!(schema.properties["smoke-small"].labels, BTreeSet::from(["seen".to_string()]));
    }

    #[test]
    fn the_dashboard_has_an_overview_and_a_group_per_property() {
        let board = dashboard("ephyra", "ephyra", &schema());
        let groups: Vec<&str> = board["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["definition"]["title"].as_str().unwrap())
            .collect();
        assert_eq!(groups, ["Overview", "Property smoke-small", "Property smoke-sorted"]);
        let titles = |group: usize| -> Vec<String> {
            board["widgets"][group]["definition"]["widgets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|w| w["definition"]["title"].as_str().map(str::to_string))
                .collect()
        };
        let sorted = titles(2);
        for wanted in [
            "Feature len (avg, p50, p90, max)",
            "Feature parity, by value",
            "Cover long: % of passing inputs",
            "Observation sum (avg, p50, p90, max)",
        ] {
            assert!(sorted.iter().any(|t| t == wanted), "{wanted} in {sorted:?}");
        }
        assert!(titles(1).iter().any(|t| t == "Labels"));
        assert!(titles(1).iter().any(|t| t == "Target score (avg, max)"));
        assert!(!sorted.iter().any(|t| t.starts_with("Target") || t.starts_with("Lines covered")));
        let text = board.to_string();
        assert!(text.contains(
            "p90:ephyra.feature{property:smoke-sorted,$run,$env,$version,feature:len,status:passed}"
        ));
        assert!(
            text.contains("sum:ephyra.cases{$property,$run,$env,$version} by {status}.as_count()")
        );
        let variables: Vec<&str> = board["template_variables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert_eq!(variables, ["property", "run", "env", "version"]);
    }

    #[test]
    fn discovery_queries_cover_every_kind() {
        let queries = discovery_queries("ephyra", "env:ci");
        assert_eq!(queries.len(), 8);
        assert!(queries.iter().all(|(_, q)| q.contains("{env:ci}") && q.contains("by {property")));
    }
}
