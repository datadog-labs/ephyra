# ephyra-datadog

Send [ephyra](https://github.com/datadog-labs/ephyra)'s test cases to Datadog. [Tyche](https://github.com/tyche-pbt/tyche-extension) shows one run's
inputs; this shows the same breakdowns as metrics over every run, so you
can watch a generator change, compare versions, and alert on them.

```rust
ephyra_datadog::Datadog::new()
    .tag("run:nightly-42")
    .log_file("ephyra-cases.jsonl", 1_000)
    .install();
```

Call it once per process, before the runs: in `main` of a long-running
runner, or in a test-only initialiser. Every later run whose property passes
`only` (all of them by default) then sends to the agent's DogStatsD port
(`DD_DOGSTATSD_URL=udp://host:port`, or `DD_AGENT_HOST` and
`DD_DOGSTATSD_PORT`, default `127.0.0.1:8125`):

| metric | type | tags |
|---|---|---|
| `ephyra.cases` | count | `status`, `origin` |
| `ephyra.feature` | distribution | `feature`, `status` |
| `ephyra.feature.value` | count | `feature`, `value`, `status` |
| `ephyra.label` | count | `label`, `status` |
| `ephyra.cover` | count | `cover`, `met`, `status` |
| `ephyra.target` | distribution | `status` |
| `ephyra.observation` | distribution | `observation`, `status` |
| `ephyra.time.generate`, `ephyra.time.execute` | distribution (seconds) | `status`, `origin` |
| `ephyra.coverage.case_lines` | distribution | `status` |
| `ephyra.coverage.lines` | gauge | |
| `ephyra.runs` | count | `result` |

All of them carry `property`, the given tags, and `env`, `service` and
`version` from `DD_ENV`, `DD_SERVICE` and `DD_VERSION`. Counts are summed and
distribution values packed in the process, and sent every 10 s and when the
run ends. A run that fails, gives up, times out or aborts sends an event
with its report. Problems reaching the agent or writing the log file become
a warning in the run's report.

`status` is Tyche's: `passed`, `failed` or `gave_up`. `origin` says where the
input came from: `random`, `explore`, `mutation`, `replay`, `shrink`,
`neighbourhood` (fault-localization mutants) or `minimal` (the shrunk
failure, run once more).

## Dashboard

With the `cli` feature, `ephyra-datadog dashboard` finds the properties,
features, labels, covers, observations, target scores and coverage that
reached Datadog, and lays out a dashboard: an overview of every property,
then a group per property with a chart for each of them. Template variables
pick the property, run, env and version.

```sh
cargo install ephyra-datadog --features cli
export DD_API_KEY=... DD_APP_KEY=... DD_SITE=datadoghq.com
ephyra-datadog dashboard > dashboard.json          # look first
ephyra-datadog dashboard --create                  # prints the new id
ephyra-datadog dashboard --update <id>             # after adding features
```

`--scope env:ci` limits it to some metrics, `--days` sets how far back to
look, and `--prefix` matches a sink given another prefix. Percentile lines
stay empty until percentiles are enabled for the distribution metrics in
Datadog (Metrics Summary, per metric).

## Logs

With `log_file(path, every)`, the failures the search found, the minimal
failure, and one in `every` of the other inputs are appended as JSON lines
with the input's `Debug` rendering, features, observations and timing. To
search them in Datadog Logs, have the agent tail the file:

```yaml
# conf.d/ephyra.d/conf.yaml
logs:
  - type: file
    path: /path/to/ephyra-cases.jsonl
    service: ephyra
    source: ephyra
```

Each line's `status` is the log level (`error` for failures, `info`
otherwise), `ddtags` carries the metric tags, and `case_status` is the
input's status.

## Cost

While the sink is open, ephyra renders every input with `Debug` and
evaluates `observe` closures, and instrumented builds record the lines each
input runs. Use `only` to leave out properties you do not want. Text
feature values become tags, so each feature keeps at most 100 distinct
values per run (`max_tag_values`) and counts the rest as `value:other`.
