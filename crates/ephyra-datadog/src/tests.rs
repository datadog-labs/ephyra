// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use std::{
    net::UdpSocket,
    time::Duration,
};

use ephyra::{
    Config,
    Status,
    TestResult,
    check,
    gens,
};

use super::*;

/// A parsed DogStatsD metric line.
#[derive(Debug)]
struct Metric {
    name: String,
    values: Vec<f64>,
    kind: String,
    tags: Vec<String>,
}

impl Metric {
    fn has(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

/// A local socket standing in for the agent's DogStatsD port.
struct FakeAgent(UdpSocket);

impl FakeAgent {
    fn new() -> FakeAgent {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        FakeAgent(socket)
    }

    fn addr(&self) -> String {
        self.0.local_addr().unwrap().to_string()
    }

    /// Every line received so far: metrics, and event lines as they came.
    fn received(&self) -> (Vec<Metric>, Vec<String>) {
        let (mut metrics, mut events) = (Vec::new(), Vec::new());
        let mut buf = vec![0; 65_536];
        while let Ok(n) = self.0.recv(&mut buf) {
            for line in std::str::from_utf8(&buf[..n]).unwrap().lines() {
                if line.starts_with("_e{") {
                    events.push(line.to_string());
                    continue;
                }
                let mut fields = line.split('|');
                let (name, values) = fields.next().unwrap().split_once(':').unwrap();
                let kind = fields.next().unwrap().to_string();
                let tags = fields
                    .find_map(|f| f.strip_prefix('#'))
                    .map(|t| t.split(',').map(str::to_string).collect())
                    .unwrap_or_default();
                let values = values.split(':').map(|v| v.parse().unwrap()).collect();
                metrics.push(Metric { name: name.to_string(), values, kind, tags });
            }
        }
        (metrics, events)
    }
}

fn sum(metrics: &[Metric], name: &str, tags: &[&str]) -> f64 {
    metrics
        .iter()
        .filter(|m| m.name == name && tags.iter().all(|t| m.has(t)))
        .flat_map(|m| m.values.iter())
        .sum()
}

fn values(metrics: &[Metric], name: &str, tags: &[&str]) -> usize {
    metrics
        .iter()
        .filter(|m| m.name == name && tags.iter().all(|t| m.has(t)))
        .map(|m| m.values.len())
        .sum()
}

fn config(name: &str) -> Config {
    Config::default().seed(7).tests(200).name(name).no_regressions()
}

#[test]
fn a_failing_run_sends_its_cases_features_event_and_log() {
    let agent = FakeAgent::new();
    let log = std::env::temp_dir().join(format!("ephyra-datadog-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&log);
    Datadog::from_env(|_| None)
        .agent(&agent.addr())
        .tag("run:Test Run")
        .only(|name| name == "dd-failing")
        .log_file(&log, 10)
        .max_tag_values(3)
        .install();

    let report = check(&config("dd-failing"), gens::int(0i32..=1000), |n: i32| {
        ephyra::feature("magnitude", n);
        ephyra::feature("digit", (n % 10).to_string());
        ephyra::label("seen");
        ephyra::cover(n % 2 == 0, "even");
        ephyra::observe("half", || n / 2);
        ephyra::target(f64::from(n));
        TestResult::check(n < 900, "too big")
    });
    assert_eq!(report.status, Status::Failed, "{report}");
    assert!(report.health.iter().all(|w| w.kind != "observations"), "{report}");
    let (metrics, events) = agent.received();

    assert!(
        metrics.iter().all(|m| m.has("property:dd-failing") && m.has("run:test_run")),
        "{metrics:?}"
    );
    let passed = report.tests as f64;
    assert_eq!(sum(&metrics, "ephyra.cases", &["status:passed", "origin:random"]), passed);
    assert_eq!(sum(&metrics, "ephyra.cases", &["status:failed", "origin:random"]), 1.0);
    assert!(sum(&metrics, "ephyra.cases", &["origin:shrink"]) > 0.0);
    assert_eq!(sum(&metrics, "ephyra.cases", &["origin:minimal"]), 1.0);
    let all_passed = sum(&metrics, "ephyra.cases", &["status:passed"]);
    let label_count = sum(&metrics, "ephyra.label", &["label:seen", "status:passed"]);
    assert_eq!(label_count, all_passed);
    let magnitudes = values(&metrics, "ephyra.feature", &["feature:magnitude", "status:passed"]);
    assert_eq!(magnitudes as f64, all_passed);
    assert!(metrics.iter().all(|m| m.name != "ephyra.feature" || m.kind == "d"));
    let digits: BTreeSet<&str> = metrics
        .iter()
        .filter(|m| m.name == "ephyra.feature.value" && m.has("feature:digit"))
        .flat_map(|m| m.tags.iter().filter_map(|t| t.strip_prefix("value:")))
        .collect();
    assert_eq!(digits.len(), 4, "three values and other: {digits:?}");
    assert!(digits.contains("other"));
    let covered = sum(&metrics, "ephyra.cover", &["cover:even", "met:true", "status:passed"])
        + sum(&metrics, "ephyra.cover", &["cover:even", "met:false", "status:passed"]);
    assert_eq!(covered, all_passed);
    assert_eq!(
        values(&metrics, "ephyra.observation", &["observation:half", "status:passed"]) as f64,
        all_passed
    );
    assert_eq!(values(&metrics, "ephyra.target", &["status:passed"]) as f64, all_passed);
    let cases = sum(&metrics, "ephyra.cases", &[]);
    assert_eq!(values(&metrics, "ephyra.time.execute", &[]) as f64, cases);
    assert_eq!(sum(&metrics, "ephyra.runs", &["result:failed"]), 1.0);
    let case_lines = values(&metrics, "ephyra.coverage.case_lines", &[]);
    let total_lines = metrics.iter().any(|m| m.name == "ephyra.coverage.lines" && m.kind == "g");
    assert_eq!(case_lines > 0, total_lines, "instrumented builds send both");
    if std::env::var_os("EPHYRA_DATADOG_EXPECT_COVERAGE").is_some() {
        assert_eq!(case_lines as f64, cases);
    }

    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].contains(":ephyra: dd-failing failed|") && events[0].contains("|t:error|"));
    assert!(events[0].contains("ephyra: FAILED") && events[0].contains("result:failed"));

    let text = std::fs::read_to_string(&log).unwrap();
    let _ = std::fs::remove_file(&log);
    let lines: Vec<serde_json::Value> =
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let logged = |status: &str| lines.iter().filter(|l| l["case_status"] == status).count();
    assert_eq!(
        logged("passed"),
        all_passed.div_euclid(10.0) as usize + usize::from(all_passed % 10.0 != 0.0)
    );
    let failures: Vec<_> = lines.iter().filter(|l| l["case_status"] == "failed").collect();
    assert_eq!(failures.len(), 2, "the first failure and the minimal one");
    let minimal = failures.last().unwrap();
    assert_eq!(minimal["origin"], "minimal");
    assert_eq!(minimal["status"], "error");
    assert_eq!(minimal["representation"], report.failure.as_ref().unwrap().counterexample().debug);
    assert_eq!(minimal["message"], "dd-failing: failed: too big");
    assert!(minimal["ddtags"].as_str().unwrap().contains("property:dd-failing"));
    assert!(minimal["features"]["magnitude"].is_number());
    assert_eq!(minimal["run_seed"], report.seed);
}

#[test]
fn a_passing_run_sends_no_event_and_other_properties_nothing() {
    let agent = FakeAgent::new();
    Datadog::from_env(|_| None).agent(&agent.addr()).only(|name| name == "dd-passing").install();
    let report = check(&config("dd-passing"), gens::int(0i32..=9), |_| true);
    assert!(report.passed());
    let skipped = check(&config("dd-elsewhere"), gens::int(0i32..=9), |_| true);
    assert!(skipped.passed());
    let (metrics, events) = agent.received();
    assert!(events.is_empty(), "{events:?}");
    assert!(metrics.iter().all(|m| m.has("property:dd-passing")));
    assert_eq!(sum(&metrics, "ephyra.cases", &["status:passed"]), report.tests as f64);
    assert_eq!(sum(&metrics, "ephyra.runs", &["result:passed"]), 1.0);
}

#[test]
fn an_unreachable_agent_is_a_report_warning() {
    Datadog::from_env(|_| None)
        .agent("no-such-agent.invalid:8125")
        .only(|name| name == "dd-unreachable")
        .install();
    let report = check(&config("dd-unreachable").tests(5), gens::int(0i32..=9), |_| true);
    assert!(report.passed());
    let warning = report.health.iter().find(|w| w.kind == "observations").expect("warned");
    assert!(warning.message.contains("cannot resolve the Datadog agent"), "{}", warning.message);
}

#[test]
fn the_environment_names_the_agent_and_unified_tags() {
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |name: &str| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string())
    };
    let default = Datadog::from_env(env(&[]));
    assert_eq!(default.agent, Ok("127.0.0.1:8125".parse().unwrap()));
    assert!(default.tags.is_empty());
    let host =
        Datadog::from_env(env(&[("DD_AGENT_HOST", "127.0.0.2"), ("DD_DOGSTATSD_PORT", "9")]));
    assert_eq!(host.agent, Ok("127.0.0.2:9".parse().unwrap()));
    let url = Datadog::from_env(env(&[
        ("DD_DOGSTATSD_URL", "udp://127.0.0.3:10"),
        ("DD_AGENT_HOST", "127.0.0.2"),
        ("DD_ENV", "Staging"),
        ("DD_VERSION", "1.2"),
    ]));
    assert_eq!(url.agent, Ok("127.0.0.3:10".parse().unwrap()));
    assert_eq!(url.tags, ["env:staging", "version:1.2"]);
    let unix = Datadog::from_env(env(&[("DD_DOGSTATSD_URL", "unix:///var/run/dsd.socket")]));
    assert!(unix.agent.unwrap_err().contains("only udp://"));
}

#[test]
fn tags_are_sanitized_like_datadog_does() {
    assert_eq!(tag_of("feature", "work (µs)"), "feature:work_µs_");
    assert_eq!(tag_of("value", "A  B\nC"), "value:a_b_c");
    assert_eq!(tag_of("origin", "random"), "origin:random");
    assert_eq!(sanitize(&"x".repeat(300)).len(), 200);
}
