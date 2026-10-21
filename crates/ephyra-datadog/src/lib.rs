//! Send ephyra's test cases to Datadog: what Tyche shows for one run, as
//! metrics over every run.
//!
//! [`Datadog`] is an [`ephyra::sink::Sink`]. Install it once per process and
//! every run sends, through the agent's DogStatsD port:
//!
//! | metric | type | tags | |
//! |---|---|---|---|
//! | `ephyra.cases` | count | `status`, `origin` | evaluated inputs (Tyche's sample breakdown) |
//! | `ephyra.feature` | distribution | `feature`, `status` | numeric [`feature`](ephyra::feature)s |
//! | `ephyra.feature.value` | count | `feature`, `value`, `status` | text features |
//! | `ephyra.label` | count | `label`, `status` | [`label`](ephyra::label)s |
//! | `ephyra.cover` | count | `cover`, `met`, `status` | [`cover`](ephyra::cover) conditions |
//! | `ephyra.target` | distribution | `status` | [`target`](ephyra::target) scores |
//! | `ephyra.observation` | distribution | `observation`, `status` | numeric [`observe`](ephyra::observe) and [`measure`](ephyra::measure) values |
//! | `ephyra.time.generate`, `ephyra.time.execute` | distribution | `status`, `origin` | seconds per input |
//! | `ephyra.coverage.case_lines` | distribution | `status` | source lines each input executed (instrumented builds) |
//! | `ephyra.coverage.lines` | gauge | | distinct lines executed so far in the run |
//! | `ephyra.runs` | count | `result` | finished runs |
//!
//! Every metric also carries `property` and the [`tag`](Datadog::tag)s
//! given, plus `env`, `service` and `version` from `DD_ENV`, `DD_SERVICE`
//! and `DD_VERSION`. A run that does not pass also sends an event with the
//! report (an error when it failed or the generator panicked, a warning when
//! it gave up or timed out). With
//! [`log_file`](Datadog::log_file), failing inputs and a sample of the rest
//! are appended as JSON lines for the agent to tail into Datadog Logs.
//!
//! ```no_run
//! ephyra_datadog::Datadog::new().tag("team:testing").install();
//! ```
//!
//! Text feature values become tags, so each feature keeps at most
//! [`max_tag_values`](Datadog::max_tag_values) distinct values per run and
//! counts the rest as `value:other`.

pub mod dashboard;
mod statsd;

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
        HashMap,
        HashSet,
    },
    fs::File,
    io::Write as _,
    net::{
        SocketAddr,
        ToSocketAddrs,
    },
    path::PathBuf,
    sync::{
        Arc,
        Mutex,
        PoisonError,
    },
    time::{
        Duration,
        Instant,
    },
};

use ephyra::{
    FeatureValue,
    Report,
    Status,
    sink::{
        CaseStatus,
        RunInfo,
        RunSink,
        Sink,
        TestCase,
    },
};

use crate::statsd::{
    Agent,
    AlertType,
    Buffer,
};

/// Which properties a [`Datadog`] sink sends.
type Filter = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A [`Sink`] that sends each run's test cases to a Datadog agent.
#[derive(Clone)]
pub struct Datadog {
    agent: Result<SocketAddr, String>,
    prefix: String,
    tags: Vec<String>,
    filter: Option<Filter>,
    log: Option<Arc<LogFile>>,
    flush_every: Duration,
    max_packet: usize,
    max_tag_values: usize,
}

impl Default for Datadog {
    fn default() -> Self {
        Datadog::new()
    }
}

impl Datadog {
    /// Send to the agent at `DD_DOGSTATSD_URL` (`udp://host:port`), else
    /// `DD_AGENT_HOST` (default `127.0.0.1`) and `DD_DOGSTATSD_PORT`
    /// (default 8125).
    pub fn new() -> Datadog {
        Datadog::from_env(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
    }

    fn from_env(var: impl Fn(&str) -> Option<String>) -> Datadog {
        let agent = match var("DD_DOGSTATSD_URL") {
            Some(url) => {
                match url.strip_prefix("udp://") {
                    Some(addr) => resolve(addr),
                    None => Err(format!("DD_DOGSTATSD_URL={url}: only udp:// is supported")),
                }
            },
            None => {
                let host = var("DD_AGENT_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
                let port = var("DD_DOGSTATSD_PORT").unwrap_or_else(|| "8125".to_string());
                resolve(&format!("{host}:{port}"))
            },
        };
        let tags = [("DD_ENV", "env"), ("DD_SERVICE", "service"), ("DD_VERSION", "version")]
            .into_iter()
            .filter_map(|(name, tag)| var(name).map(|value| tag_of(tag, &value)))
            .collect();
        Datadog {
            agent,
            prefix: "ephyra".to_string(),
            tags,
            filter: None,
            log: None,
            flush_every: Duration::from_secs(10),
            max_packet: 1432,
            max_tag_values: 100,
        }
    }

    /// Send to the agent at `addr` (a `host:port`) instead.
    pub fn agent(mut self, addr: &str) -> Self {
        self.agent = resolve(addr);
        self
    }

    /// Start metric names with `prefix` instead of `ephyra`.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Add `tag` (`key:value`, or a bare value) to every metric, event and
    /// log line, such as the run or the version under test.
    pub fn tag(mut self, tag: impl AsRef<str>) -> Self {
        self.tags.push(sanitize(tag.as_ref()));
        self
    }

    /// Send only the properties whose name `keep` accepts.
    pub fn only(mut self, keep: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.filter = Some(Arc::new(keep));
        self
    }

    /// Also append inputs to `path` as JSON lines: every failure the search
    /// found, the minimal failure (but not the shrink candidates and
    /// mutants run on the way to it), and one in `every` of the
    /// passing and discarded inputs (0 for none of them). Point the agent's
    /// log collection at the file to search inputs in Datadog Logs.
    pub fn log_file(mut self, path: impl Into<PathBuf>, every: u64) -> Self {
        self.log = Some(Arc::new(LogFile { path: path.into(), every, file: Mutex::new(None) }));
        self
    }

    /// Send buffered values at least this often while a run goes on (default
    /// 10 s); a run's remaining values go when it finishes.
    pub fn flush_every(mut self, every: Duration) -> Self {
        self.flush_every = every;
        self
    }

    /// The largest datagram to send (default 1 432 bytes, which fits an
    /// Ethernet frame; the agent accepts up to its `dogstatsd_buffer_size`).
    pub fn max_packet(mut self, bytes: usize) -> Self {
        self.max_packet = bytes;
        self
    }

    /// The most distinct values a text feature sends as tags in one run
    /// (default 100); later values count as `value:other`.
    pub fn max_tag_values(mut self, values: usize) -> Self {
        self.max_tag_values = values;
        self
    }

    /// Install this sink for every later run in the process.
    pub fn install(self) {
        ephyra::sink::install(self);
    }
}

fn resolve(addr: &str) -> Result<SocketAddr, String> {
    match addr.to_socket_addrs() {
        Ok(mut addrs) => addrs.next().ok_or_else(|| format!("{addr} resolves to no address")),
        Err(e) => Err(format!("cannot resolve the Datadog agent at {addr}: {e}")),
    }
}

impl Sink for Datadog {
    fn open(&self, run: &RunInfo<'_>) -> Option<Box<dyn RunSink>> {
        if self.filter.as_ref().is_some_and(|keep| !keep(run.property)) {
            return None;
        }
        let agent = self.agent.clone().and_then(|addr| {
            Agent::connect(addr, self.max_packet)
                .map_err(|e| format!("cannot open a socket to the Datadog agent at {addr}: {e}"))
        });
        let mut tags = self.tags.clone();
        tags.push(tag_of("property", run.property));
        Some(Box::new(Recorder {
            prefix: self.prefix.clone(),
            property: run.property.to_string(),
            seed: run.seed,
            tags,
            agent,
            buffer: Buffer::default(),
            flushed: Instant::now(),
            flush_every: self.flush_every,
            max_line: self.max_packet,
            max_tag_values: self.max_tag_values,
            tag_values: HashMap::new(),
            lines: BTreeMap::new(),
            log: self.log.clone(),
            logged: HashMap::new(),
            log_error: None,
        }))
    }
}

/// The file sampled inputs go to, opened on the first line.
struct LogFile {
    path: PathBuf,
    every: u64,
    file: Mutex<Option<File>>,
}

impl LogFile {
    fn append(&self, line: &str) -> std::io::Result<()> {
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if file.is_none() {
            *file = Some(std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?);
        }
        let file = file.as_mut().expect("opened above");
        file.write_all(format!("{line}\n").as_bytes())
    }
}

/// One run's metrics on their way to the agent.
struct Recorder {
    prefix: String,
    property: String,
    seed: u64,
    /// The configured tags and `property`.
    tags: Vec<String>,
    agent: Result<Agent, String>,
    buffer: Buffer,
    flushed: Instant,
    flush_every: Duration,
    max_line: usize,
    max_tag_values: usize,
    /// The values each text feature has sent as tags.
    tag_values: HashMap<String, HashSet<String>>,
    /// Lines executed so far, by file.
    lines: BTreeMap<String, BTreeSet<u32>>,
    log: Option<Arc<LogFile>>,
    /// Inputs seen per status, for sampling the log.
    logged: HashMap<CaseStatus, u64>,
    log_error: Option<String>,
}

impl Recorder {
    fn name(&self, metric: &str) -> String {
        format!("{}.{metric}", self.prefix)
    }

    /// Add `metric` with the run's tags and `extra`.
    fn count(&mut self, metric: &str, extra: &[&str]) {
        let name = self.name(metric);
        let tags = self.with(extra);
        self.buffer.count(&name, &tags.iter().map(String::as_str).collect::<Vec<_>>(), 1);
    }

    fn distribution(&mut self, metric: &str, extra: &[&str], value: f64) {
        let name = self.name(metric);
        let tags = self.with(extra);
        self.buffer.distribution(
            &name,
            &tags.iter().map(String::as_str).collect::<Vec<_>>(),
            value,
        );
    }

    fn with(&self, extra: &[&str]) -> Vec<String> {
        self.tags.iter().cloned().chain(extra.iter().map(|t| t.to_string())).collect()
    }

    /// `value` as a tag value for text feature `feature`, or `other` once
    /// the feature has sent `max_tag_values` others.
    fn tag_value(&mut self, feature: &str, value: &str) -> String {
        let value = sanitize(value);
        let seen = self.tag_values.entry(feature.to_string()).or_default();
        if seen.contains(&value) || seen.len() < self.max_tag_values {
            seen.insert(value.clone());
            value
        } else {
            "other".to_string()
        }
    }

    fn flush(&mut self) {
        if !self.lines.is_empty() {
            let name = self.name("coverage.lines");
            let total = self.lines.values().map(BTreeSet::len).sum::<usize>() as f64;
            let tags: Vec<&str> = self.tags.iter().map(String::as_str).collect();
            self.buffer.gauge(&name, &tags, total);
        }
        let lines = self.buffer.drain(self.max_line);
        if let Ok(agent) = &mut self.agent {
            agent.send(lines);
        }
        self.flushed = Instant::now();
    }

    fn log(&mut self, case: &TestCase<'_>) {
        let Some(log) = self.log.clone() else { return };
        let keep = match case.status {
            // Shrink candidates and fault-localization mutants of the
            // failure are counted, not logged; the minimal failure is.
            CaseStatus::Failed => !matches!(case.origin, "shrink" | "neighbourhood"),
            _ => {
                let seen = self.logged.entry(case.status).or_default();
                *seen += 1;
                log.every > 0 && (*seen - 1).is_multiple_of(log.every)
            },
        };
        if !keep {
            return;
        }
        let values = |pairs: &[(String, FeatureValue)]| {
            pairs
                .iter()
                .map(|(name, value)| {
                    let value = match value {
                        FeatureValue::Number(n) => serde_json::json!(n),
                        FeatureValue::Text(t) => serde_json::json!(t),
                    };
                    (name.clone(), value)
                })
                .collect::<serde_json::Map<_, _>>()
        };
        let line = serde_json::json!({
            "timestamp": now_millis(),
            "status": if case.status == CaseStatus::Failed { "error" } else { "info" },
            "message": if case.status_reason.is_empty() {
                format!("{}: {}", self.property, case.status.as_str())
            } else {
                format!("{}: {}: {}", self.property, case.status.as_str(), case.status_reason)
            },
            "ddtags": self.tags.join(","),
            "property": self.property,
            "case_status": case.status.as_str(),
            "status_reason": case.status_reason,
            "origin": case.origin,
            "representation": truncate(case.representation, 16_000),
            "labels": case.labels,
            "features": values(case.features),
            "observations": values(case.observations),
            "covers": case.covers.iter().map(|(label, met)| (label.clone(), (*met).into())).collect::<serde_json::Map<_, _>>(),
            "target": case.score,
            "seed": case.seed,
            "run_seed": self.seed,
            "iteration": case.iteration,
            "size": case.size,
            "generate_s": case.generate.as_secs_f64(),
            "execute_s": case.execute.as_secs_f64(),
            "shrink_accepted": case.shrink_accepted,
        });
        if let Err(e) = log.append(&line.to_string()) {
            self.log_error = Some(format!("could not append to {}: {e}", log.path.display()));
        }
    }
}

impl RunSink for Recorder {
    fn test_case(&mut self, case: &TestCase<'_>) {
        let status = format!("status:{}", case.status.as_str());
        let origin = tag_of("origin", case.origin);
        self.count("cases", &[&status, &origin]);
        for label in case.labels {
            self.count("label", &[&tag_of("label", label), &status]);
        }
        for (name, value) in case.features {
            let feature = tag_of("feature", name);
            match value {
                FeatureValue::Number(n) => self.distribution("feature", &[&feature, &status], *n),
                FeatureValue::Text(t) => {
                    let value = format!("value:{}", self.tag_value(name, t));
                    self.count("feature.value", &[&feature, &value, &status]);
                },
            }
        }
        for (label, met) in case.covers {
            self.count(
                "cover",
                &[&tag_of("cover", label), if *met { "met:true" } else { "met:false" }, &status],
            );
        }
        if let Some(score) = case.score {
            self.distribution("target", &[&status], score);
        }
        for (name, value) in case.observations {
            if let FeatureValue::Number(n) = value {
                self.distribution("observation", &[&tag_of("observation", name), &status], *n);
            }
        }
        self.distribution("time.generate", &[&status, &origin], case.generate.as_secs_f64());
        self.distribution("time.execute", &[&status, &origin], case.execute.as_secs_f64());
        if let Some(coverage) = case.coverage {
            let executed = coverage.values().map(Vec::len).sum::<usize>();
            self.distribution("coverage.case_lines", &[&status], executed as f64);
            for (file, lines) in coverage {
                self.lines.entry(file.clone()).or_default().extend(lines);
            }
        }
        self.log(case);
        if self.buffer.len() >= 50_000 || self.flushed.elapsed() >= self.flush_every {
            self.flush();
        }
    }

    fn finish(&mut self, report: &Report) -> Result<(), String> {
        let result = match report.status {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::GaveUp => "gave_up",
            Status::TimedOut => "timed_out",
            Status::Aborted => "aborted",
        };
        self.count("runs", &[&format!("result:{result}")]);
        self.flush();
        let alert = match report.status {
            Status::Passed => None,
            Status::Failed | Status::Aborted => Some(AlertType::Error),
            Status::GaveUp | Status::TimedOut => Some(AlertType::Warning),
        };
        if let Some(alert) = alert {
            let title = format!("ephyra: {} {}", self.property, result.replace('_', " "));
            let report = report.to_string();
            let tags = self.with(&[&format!("result:{result}")]);
            let event = statsd::event(
                &title,
                truncate(&report, 3_500),
                alert,
                &tags.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            if let Ok(agent) = &mut self.agent {
                agent.send([event]);
            }
        }
        let mut problems = Vec::new();
        match &self.agent {
            Err(e) => problems.push(e.clone()),
            Ok(Agent { failed: Some((n, e)), .. }) => {
                problems
                    .push(format!("{n} datagrams to the Datadog agent failed, the last with: {e}"))
            },
            Ok(_) => {},
        }
        problems.extend(self.log_error.take());
        if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
    }
}

/// `key:value` with `value` made a valid tag.
fn tag_of(key: &str, value: &str) -> String {
    sanitize(&format!("{key}:{value}"))
}

/// `tag` as Datadog stores it: lowercase, characters outside letters,
/// digits and `_-:./` replaced with `_`, runs of `_` collapsed, at most 200
/// characters.
fn sanitize(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    for c in tag.chars().flat_map(char::to_lowercase) {
        let c = if c.is_alphanumeric() || "_-:./".contains(c) { c } else { '_' };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    truncate(&out, 200).to_string()
}

/// At most `max` bytes of `s`, cut at a character boundary.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn now_millis() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests;
