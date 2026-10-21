//! `ephyra visualize`: a local page for evolving properties.
//!
//! Point it at the directory the runs write to (`.ephyra` by default)
//! and it serves the coverage grid pages found in `coverage/`, the latest
//! report per property from `reports.jsonl`, the targets in
//! `targets.jsonl`, and the tests it has discovered. From the page, any
//! property or test can be run again: the server starts `cargo` for it
//! with the coverage outputs pointed back at the directory, streams the
//! output, and the pages and reports refresh as the run writes them. A
//! grid page served this way gains "Run" and "Add to targets.jsonl"
//! buttons, so the loop is: pick a region, add it as a target, rewrite
//! the generator (or let an agent watching `targets.jsonl` do it), run,
//! see whether the target was reached.
//!
//! ```text
//! cargo run -p ephyra-cli -- visualize --cargo-args "--features coverage"
//! # then open http://127.0.0.1:7878/ and press "Discover tests" or "Run all"
//! ```
//!
//! Runs repeat the command the report's [`RunSpec`] records, in the
//! environment `run_env` builds: every artifact path pointed into the
//! directory, `-C instrument-coverage` unless `--no-instrument`, and
//! [`Options::resolved_target_dir`]. Start the server inside the workspace
//! so `-p` resolves. "Discover tests" builds the test binaries with `cargo
//! test --no-run --message-format=json` and lists their tests with `--list`,
//! into `tests.json`.
//!
//! The server binds to `127.0.0.1` only and speaks plain HTTP/1.1 from
//! the standard library. The only thing it ever starts is `cargo`, on
//! request, one run at a time; cross-site requests are refused by origin.

use std::{
    collections::{
        BTreeMap,
        hash_map::DefaultHasher,
    },
    fs,
    hash::{
        Hash,
        Hasher,
    },
    io::{
        self,
        BufRead,
        BufReader,
        Read,
        Write,
    },
    net::{
        SocketAddr,
        TcpListener,
        TcpStream,
    },
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Stdio,
    },
    sync::{
        Arc,
        Mutex,
        MutexGuard,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    thread::{
        self,
        JoinHandle,
    },
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use serde_json::{
    Value,
    json,
};

use ephyra::{
    config::{
        CoverageTarget,
        load_targets,
    },
    report::RunSpec,
};

/// Where to look, where to listen, and how to run cargo.
#[derive(Clone, Debug)]
pub struct Options {
    /// The directory test runs write to. Default `.ephyra`.
    pub dir: PathBuf,
    /// TCP port on 127.0.0.1; 0 picks a free one. Default 7878.
    pub port: u16,
    /// Extra arguments for every cargo invocation, e.g. `--features coverage`.
    pub cargo_args: Vec<String>,
    /// Append `-C instrument-coverage` to `RUSTFLAGS` for runs. Default true.
    pub instrument: bool,
    /// `CARGO_TARGET_DIR` for runs. Default: the environment's, else
    /// `target/cov`, so instrumented builds do not evict the normal ones.
    pub target_dir: Option<PathBuf>,
    /// The cargo executable. Default `cargo`.
    pub cargo: PathBuf,
    /// Additional environment variables applied to every Cargo run.
    pub environment: Vec<(String, String)>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dir: PathBuf::from(".ephyra"),
            port: 7878,
            cargo_args: Vec::new(),
            instrument: true,
            target_dir: None,
            cargo: PathBuf::from("cargo"),
            environment: Vec::new(),
        }
    }
}

impl Options {
    /// The `CARGO_TARGET_DIR` runs use: [`target_dir`](Self::target_dir),
    /// else the environment's, else `target/cov`.
    pub fn resolved_target_dir(&self) -> PathBuf {
        self.target_dir
            .clone()
            .or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("target/cov"))
    }
}

/// A running server; dropping it asks the accept loop to stop. A cargo
/// run in progress is left to finish.
pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// The bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The page's URL.
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Stop accepting and wait for the accept loop.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The output of the current (or last) cargo run.
struct RunLog {
    id: u64,
    name: String,
    command: String,
    started: u64,
    finished: Option<u64>,
    status: Option<i32>,
    lines: Vec<String>,
    dropped: u64,
}

const MAX_LOG_LINES: usize = 4000;

impl RunLog {
    fn running(&self) -> bool {
        self.finished.is_none()
    }

    fn push(&mut self, line: String) {
        if self.lines.len() >= MAX_LOG_LINES {
            self.lines.drain(..MAX_LOG_LINES / 4);
            self.dropped += (MAX_LOG_LINES / 4) as u64;
        }
        self.lines.push(line);
    }

    fn json(&self, with_lines: bool) -> Value {
        let mut value = json!({
            "id": self.id,
            "name": self.name,
            "command": self.command,
            "started": self.started,
            "finished": self.finished,
            "status": self.status,
            "running": self.running(),
            "line_count": self.lines.len() as u64 + self.dropped,
        });
        if with_lines {
            value["lines"] = json!(self.lines);
            value["dropped"] = json!(self.dropped);
        }
        value
    }
}

struct State {
    options: Options,
    stop: Arc<AtomicBool>,
    run: Mutex<Option<RunLog>>,
    /// Workspace members and their features, from `cargo metadata`, read
    /// on first use.
    members: Mutex<Option<Vec<Member>>>,
}

impl State {
    fn dir(&self) -> &Path {
        &self.options.dir
    }

    fn log(&self) -> MutexGuard<'_, Option<RunLog>> {
        self.run.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn members(&self) -> Vec<Member> {
        let mut cached = self.members.lock().unwrap_or_else(|e| e.into_inner());
        cached.get_or_insert_with(|| workspace_members(&self.options.cargo)).clone()
    }
}

/// A workspace member and the features it declares.
/// A workspace package, as `cargo metadata` describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Member {
    pub name: String,
    pub features: Vec<String>,
    /// The directory of its `Cargo.toml`, where its tests run.
    pub manifest_dir: PathBuf,
    pub targets: Vec<Target>,
}

/// A build target of a package: its first kind (`lib`, `test`, `bin`, ...),
/// its name and its root source file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Target {
    pub kind: String,
    pub name: String,
    pub src_path: PathBuf,
}

/// The workspace's packages, from `cargo metadata` run in the current
/// directory; empty when cargo fails.
pub(crate) fn workspace_members(cargo: &Path) -> Vec<Member> {
    Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|out| serde_json::from_slice::<Value>(&out.stdout).ok())
        .and_then(|metadata| {
            Some(
                metadata
                    .get("packages")?
                    .as_array()?
                    .iter()
                    .filter_map(|package| {
                        let manifest = PathBuf::from(package.get("manifest_path")?.as_str()?);
                        Some(Member {
                            name: package.get("name")?.as_str()?.to_string(),
                            features: package
                                .get("features")?
                                .as_object()?
                                .keys()
                                .cloned()
                                .collect(),
                            manifest_dir: manifest.parent()?.to_path_buf(),
                            targets: package
                                .get("targets")?
                                .as_array()?
                                .iter()
                                .filter_map(|target| {
                                    Some(Target {
                                        kind: target.get("kind")?.get(0)?.as_str()?.to_string(),
                                        name: target.get("name")?.as_str()?.to_string(),
                                        src_path: PathBuf::from(target.get("src_path")?.as_str()?),
                                    })
                                })
                                .collect(),
                        })
                    })
                    .collect(),
            )
        })
        .unwrap_or_default()
}

/// The extra cargo arguments for a run of `package` (or of the whole
/// workspace): `--features` values a selected package does not declare are
/// dropped, since cargo refuses them, and each drop is explained.
pub(crate) fn cargo_args_for(
    package: Option<&str>,
    given: &[String],
    members: &[Member],
) -> (Vec<String>, Vec<String>) {
    let mut args = Vec::new();
    let mut features = Vec::new();
    let mut it = given.iter();
    while let Some(arg) = it.next() {
        let value = if arg == "--features" || arg == "-F" {
            it.next().map(String::as_str)
        } else if let Some(value) = arg.strip_prefix("--features=") {
            Some(value)
        } else {
            args.push(arg.clone());
            continue;
        };
        features.extend(value.unwrap_or_default().split([',', ' ']).filter(|f| !f.is_empty()));
    }
    let (kept, dropped): (Vec<&str>, Vec<&str>) = features.into_iter().partition(|feature| {
        let (owner, name) = feature.split_once('/').map_or((None, *feature), |(o, n)| (Some(o), n));
        let known = members.iter().any(|m| owner.is_none_or(|o| m.name == o));
        !known
            || members
                .iter()
                .filter(|m| package.is_none_or(|p| m.name == p))
                .filter(|m| owner.is_none_or(|o| m.name == o))
                .any(|m| m.features.iter().any(|f| f == name))
    });
    let notes = dropped
        .iter()
        .map(|feature| {
            format!(
                "  dropped --features {feature}: {} has no such feature",
                package.unwrap_or("no selected package")
            )
        })
        .collect();
    if !kept.is_empty() {
        args.push("--features".to_string());
        args.push(kept.join(","));
    }
    (args, notes)
}

/// Start serving in a background thread.
pub fn start(options: Options) -> io::Result<Server> {
    let listener = TcpListener::bind(("127.0.0.1", options.port))?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(State {
        options,
        stop: stop.clone(),
        run: Mutex::new(None),
        members: Mutex::new(None),
    });
    let thread = thread::spawn(move || {
        while !state.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let state = state.clone();
                    thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                        let _ = handle(stream, &state);
                    });
                },
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                },
                Err(_) => break,
            }
        }
    });
    Ok(Server { addr, stop, thread: Some(thread) })
}

struct Request {
    method: String,
    path: String,
    origin: Option<String>,
    body: Vec<u8>,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    let mut origin = None;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = lower.strip_prefix("origin:") {
            origin = Some(value.trim().to_string());
        }
    }
    if content_length > 1 << 20 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    Ok(Some(Request { method, path, origin, body }))
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_json(stream: &mut TcpStream, status: u16, value: Value) -> io::Result<()> {
    respond(stream, status, "application/json", value.to_string().as_bytes())
}

/// A browser sends `Origin` on every cross-site POST; only the page itself
/// (same origin) and non-browser clients (no header) may change state.
fn same_origin(request: &Request) -> bool {
    match &request.origin {
        None => true,
        Some(origin) => {
            origin.starts_with("http://127.0.0.1:") || origin.starts_with("http://localhost:")
        },
    }
}

fn handle(stream: TcpStream, state: &Arc<State>) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut stream = stream;
    let Some(request) = read_request(&mut reader)? else {
        return Ok(());
    };
    if request.method == "POST" && !same_origin(&request) {
        return respond(&mut stream, 403, "text/plain", b"cross-site requests are refused");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => respond(&mut stream, 200, "text/html; charset=utf-8", INDEX.as_bytes()),
        ("GET", "/api/state") => respond_json(&mut stream, 200, snapshot(state)),
        ("GET", "/api/run") => {
            let log = state.log().as_ref().map_or(Value::Null, |log| log.json(true));
            respond_json(&mut stream, 200, log)
        },
        ("GET", "/api/events") => events(stream, state),
        ("GET", path) if path.starts_with("/grid/") => {
            let name = &path["/grid/".len()..];
            let safe = !name.is_empty()
                && !name.contains(['/', '\\'])
                && !name.contains("..")
                && name.ends_with(".html");
            match safe.then(|| fs::read(state.dir().join("coverage").join(name))) {
                Some(Ok(page)) => respond(&mut stream, 200, "text/html; charset=utf-8", &page),
                _ => respond(&mut stream, 404, "text/plain", b"no such page"),
            }
        },
        ("POST", path @ ("/api/run" | "/api/discover")) => {
            let started = if path == "/api/run" {
                run_request(state, &request.body).and_then(|(name, spec)| {
                    let package = spec.as_ref().and_then(|s| s.package.as_deref());
                    let (extra, notes) =
                        cargo_args_for(package, &state.options.cargo_args, &state.members());
                    let args = run_args(spec.as_ref(), &extra)?;
                    start_run(state, &name, args, notes, None)
                })
            } else {
                *state.members.lock().unwrap_or_else(|e| e.into_inner()) = None;
                let (extra, notes) =
                    cargo_args_for(None, &state.options.cargo_args, &state.members());
                let mut args = vec!["test".to_string()];
                args.extend(extra);
                args.extend(["--no-run", "--message-format=json"].map(String::from));
                start_run(state, "discover tests", args, notes, Some(Box::new(finish_discovery)))
            };
            match started {
                Ok(id) => respond_json(&mut stream, 202, json!({ "id": id })),
                Err(Problem(status, message)) => {
                    respond_json(&mut stream, status, json!({ "error": message }))
                },
            }
        },
        ("POST", "/api/targets") => {
            match parse_target(&request.body) {
                Ok(target) => {
                    let path = state.dir().join("targets.jsonl");
                    fs::create_dir_all(state.dir())?;
                    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
                    writeln!(file, "{}", serde_json::to_string(&target).unwrap_or_default())?;
                    let count = load_targets(&path).map(|t| t.len()).unwrap_or(0);
                    respond_json(&mut stream, 200, json!({ "targets": count }))
                },
                Err(e) => respond(&mut stream, 400, "text/plain", e.as_bytes()),
            }
        },
        ("POST", "/api/targets/clear") => {
            fs::create_dir_all(state.dir())?;
            fs::write(state.dir().join("targets.jsonl"), "")?;
            respond_json(&mut stream, 200, json!({ "targets": 0 }))
        },
        ("GET", _) => respond(&mut stream, 404, "text/plain", b"not found"),
        _ => respond(&mut stream, 405, "text/plain", b"method not allowed"),
    }
}

fn parse_target(body: &[u8]) -> Result<CoverageTarget, String> {
    let target: CoverageTarget = serde_json::from_slice(body)
        .map_err(|e| format!("expected {{\"file\",\"begin_line\",\"end_line\"}}: {e}"))?;
    if target.file.trim().is_empty() || target.file.contains(['\n', '\r']) {
        return Err("file must be a single non-empty line".to_string());
    }
    if target.begin_line == 0 || target.begin_line > target.end_line {
        return Err("lines must satisfy 1 <= begin_line <= end_line".to_string());
    }
    Ok(target)
}

// ---- running cargo ---------------------------------------------------------

/// Why a run was not started, and the HTTP status that says so.
struct Problem(u16, String);

impl From<String> for Problem {
    fn from(message: String) -> Self {
        Problem(400, message)
    }
}

/// What a `POST /api/run` body asks for: a property by report name, an
/// explicit spec (as `tests.json` lists them), or everything (`{}`).
fn run_request(state: &State, body: &[u8]) -> Result<(String, Option<RunSpec>), Problem> {
    let value: Value = if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        serde_json::from_slice(body).map_err(|e| format!("expected a JSON object: {e}"))?
    };
    if let Some(name) = value.get("name").and_then(Value::as_str) {
        let reports = latest_reports(state.dir());
        let report = reports
            .get(name)
            .ok_or_else(|| format!("no report named `{name}` in reports.jsonl"))?;
        let spec: RunSpec = serde_json::from_value(report.get("run").cloned().unwrap_or(Value::Null))
            .map_err(|_| format!("the report for `{name}` does not say how it was run; run it once under cargo with this version"))?;
        return Ok((name.to_string(), Some(spec)));
    }
    if value.get("kind").is_some() {
        let spec: RunSpec = serde_json::from_value(value.clone())
            .map_err(|e| format!("expected {{package, kind, target, test}}: {e}"))?;
        let name =
            spec.test.clone().or_else(|| spec.target.clone()).unwrap_or_else(|| "run".to_string());
        return Ok((name, Some(spec)));
    }
    Ok(("all tests".to_string(), None))
}

/// The cargo arguments for a spec (`None` runs every test).
fn run_args(spec: Option<&RunSpec>, extra: &[String]) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = match spec {
        None => ["test", "--tests"].map(String::from).into(),
        Some(spec) => {
            let package = spec
                .package
                .as_deref()
                .filter(|p| !p.is_empty())
                .ok_or("the report does not say which package ran it")?;
            if spec.kind == "example" {
                let target = spec
                    .target
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .ok_or("the report does not name the example")?;
                ["run", "-p", package, "--example", target].map(String::from).into()
            } else {
                ["test", "-p", package, "--tests"].map(String::from).into()
            }
        },
    };
    args.extend_from_slice(extra);
    if let Some(test) = spec
        .filter(|s| s.kind != "example")
        .and_then(|s| s.test.as_deref())
        .filter(|t| !t.is_empty())
    {
        args.extend(["--", test, "--exact"].map(String::from));
    }
    Ok(args)
}

/// `path`, resolved against `base` when relative.
pub(crate) fn absolute(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { base.join(path) }
}

/// The environment every run gets, as `(name, value)` for the log.
pub(crate) fn run_env(options: &Options) -> Vec<(String, String)> {
    // An unknown working directory leaves the paths relative, as given.
    let cwd = std::env::current_dir().unwrap_or_default();
    let dir = absolute(&options.dir, &cwd);
    let mut env = vec![
        ("EPHYRA_COVERAGE_HTML".to_string(), dir.join("coverage").display().to_string()),
        ("EPHYRA_COVERAGE_TARGETS".to_string(), dir.join("targets.jsonl").display().to_string()),
        ("EPHYRA_OBSERVATIONS".to_string(), dir.join("observations").display().to_string()),
        ("EPHYRA_REGRESSIONS_DIR".to_string(), dir.join("regressions").display().to_string()),
        ("EPHYRA_REPORT_JSON".to_string(), dir.join("reports.jsonl").display().to_string()),
        ("RUST_TEST_THREADS".to_string(), "1".to_string()),
        ("CARGO_TERM_COLOR".to_string(), "never".to_string()),
    ];
    if options.instrument {
        env.push(("RUSTFLAGS".to_string(), rustflags(true)));
        env.push(("EPHYRA_PROFRAW_DIR".to_string(), dir.join("profraw").display().to_string()));
    }
    env.push((
        "CARGO_TARGET_DIR".to_string(),
        absolute(&options.resolved_target_dir(), &cwd).display().to_string(),
    ));
    env.extend(options.environment.iter().cloned());
    env
}

/// The inherited `RUSTFLAGS`, with `-C instrument-coverage` appended when
/// `instrument` is set and it is not already there.
pub(crate) fn rustflags(instrument: bool) -> String {
    let mut flags = std::env::var("RUSTFLAGS").unwrap_or_default();
    if instrument && !flags.contains("instrument-coverage") {
        if !flags.is_empty() {
            flags.push(' ');
        }
        flags.push_str("-C instrument-coverage");
    }
    flags
}

/// Run `cargo test` in the foreground with reports, observations, coverage
/// pages, targets and regressions routed through [`Options::dir`]. Extra
/// cargo and test-harness arguments come from [`Options::cargo_args`].
///
/// This is the non-HTTP half of the local workflow used by the `ephyra
/// test` command. The child inherits stdin, stdout and stderr.
pub fn run_tests(options: &Options) -> io::Result<std::process::ExitStatus> {
    fs::create_dir_all(&options.dir)?;
    Command::new(&options.cargo)
        .arg("test")
        .args(&options.cargo_args)
        .envs(run_env(options))
        .status()
}

type StdoutHandler = Box<dyn FnOnce(&Arc<State>, u64, String) + Send>;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn push_line(state: &State, id: u64, line: String) {
    if let Some(log) = state.log().as_mut()
        && log.id == id
    {
        log.push(line);
    }
}

/// Start `cargo <args>` in the background, one run at a time. Its stderr
/// streams into the log; so does stdout, unless `on_stdout` wants it whole
/// when the process ends.
fn start_run(
    state: &Arc<State>,
    name: &str,
    args: Vec<String>,
    notes: Vec<String>,
    on_stdout: Option<StdoutHandler>,
) -> Result<u64, Problem> {
    let mut slot = state.log();
    if let Some(log) = slot.as_ref()
        && log.running()
    {
        return Err(Problem(409, format!("`{}` is still running", log.name)));
    }
    let id = slot.as_ref().map_or(1, |log| log.id + 1);
    let env = run_env(&state.options);
    let display = command_line(&state.options.cargo, &args);
    let mut child = Command::new(&state.options.cargo)
        .args(&args)
        .envs(env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start `{display}`: {e}"))?;
    let mut log = RunLog {
        id,
        name: name.to_string(),
        command: display.clone(),
        started: now(),
        finished: None,
        status: None,
        lines: Vec::new(),
        dropped: 0,
    };
    log.push(format!("$ {display}"));
    log.push(format!(
        "  with {}",
        env.iter().map(|(k, v)| format!("{k}={}", shell_quote(v))).collect::<Vec<_>>().join(" ")
    ));
    for note in notes {
        log.push(note);
    }
    *slot = Some(log);
    drop(slot);

    let err_thread = pump(state, id, child.stderr.take(), false);
    let out_thread = pump(state, id, child.stdout.take(), on_stdout.is_some());
    let state = state.clone();
    thread::spawn(move || {
        let _ = err_thread.join();
        let collected = out_thread.join().unwrap_or_default();
        let status = child.wait().ok().and_then(|s| s.code());
        if let Some(handler) = on_stdout {
            handler(&state, id, collected);
        }
        if let Some(log) = state.log().as_mut()
            && log.id == id
        {
            log.push(match status {
                Some(0) => "finished: ok".to_string(),
                Some(code) => format!("finished: exit code {code}"),
                None => "finished: stopped by a signal".to_string(),
            });
            log.status = status;
            log.finished = Some(now());
        }
    });
    Ok(id)
}

/// Stream `pipe` into run `id`'s log line by line, or, with `collect`, into
/// the string the thread returns.
fn pump(
    state: &Arc<State>,
    id: u64,
    pipe: Option<impl Read + Send + 'static>,
    collect: bool,
) -> JoinHandle<String> {
    let state = state.clone();
    thread::spawn(move || {
        let mut collected = String::new();
        if let Some(pipe) = pipe {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if collect {
                    collected.push_str(&line);
                    collected.push('\n');
                } else {
                    push_line(&state, id, line);
                }
            }
        }
        collected
    })
}

/// `cargo` and `args` as one shell command line, quoted where needed.
pub fn command_line(cargo: &Path, args: &[String]) -> String {
    format!(
        "{} {}",
        cargo.display(),
        args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
    )
}

fn shell_quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '"', '\'', '$', '&', '|', ';', '<', '>', '(', ')']) {
        format!("'{}'", arg.replace('\'', "'\\''"))
    } else {
        arg.to_string()
    }
}

// ---- discovering tests -----------------------------------------------------

/// A test or example binary cargo built.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Artifact {
    pub package: String,
    pub kind: String,
    pub name: String,
    pub executable: PathBuf,
    /// Built with the test harness (`profile.test`).
    pub test: bool,
}

/// The binaries in `cargo --message-format=json` output.
pub(crate) fn parse_artifacts(stdout: &str) -> Vec<Artifact> {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value.get("reason").and_then(Value::as_str) == Some("compiler-artifact"))
        .filter_map(|value| {
            let executable = value.get("executable")?.as_str()?.to_string();
            let target = value.get("target")?;
            let kind = target.get("kind")?.get(0)?.as_str()?.to_string();
            let name = target.get("name")?.as_str()?.to_string();
            let test = value.get("profile").and_then(|p| p.get("test")).and_then(Value::as_bool);
            let package = package_name(value.get("package_id")?.as_str()?);
            Some(Artifact {
                package,
                kind,
                name,
                executable: PathBuf::from(executable),
                test: test.unwrap_or(false),
            })
        })
        .collect()
}

/// The package name in a cargo package id: `path+file:///…/name#0.1.0`, or
/// `…#name@0.1.0` when the name is not the last path segment.
fn package_name(id: &str) -> String {
    let (base, fragment) = id.rsplit_once('#').unwrap_or((id, ""));
    match fragment.split_once('@') {
        Some((name, _)) => name.to_string(),
        None => base.trim_end_matches('/').rsplit('/').next().unwrap_or(base).to_string(),
    }
}

/// The tests a test binary lists (`--list --format terse`).
pub(crate) fn list_tests(executable: &Path) -> io::Result<Vec<String>> {
    let output = Command::new(executable)
        .args(["--list", "--format", "terse"])
        .env("RUST_TEST_THREADS", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    Ok(parse_test_list(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_test_list(stdout: &str) -> Vec<String> {
    stdout.lines().filter_map(|line| line.strip_suffix(": test")).map(str::to_string).collect()
}

/// After `cargo test --no-run --message-format=json`: list every test binary's
/// tests and every example into `tests.json`.
fn finish_discovery(state: &Arc<State>, id: u64, stdout: String) {
    let mut entries = Vec::new();
    let mut total = 0usize;
    for artifact in parse_artifacts(&stdout) {
        let (kind, tests) = if artifact.test {
            let tests = list_tests(&artifact.executable).unwrap_or_default();
            if tests.is_empty() {
                continue;
            }
            push_line(
                state,
                id,
                format!("{} ({}): {} tests", artifact.name, artifact.kind, tests.len()),
            );
            total += tests.len();
            ("test", tests)
        } else if artifact.kind == "example" {
            push_line(state, id, format!("{} (example)", artifact.name));
            ("example", Vec::new())
        } else {
            continue;
        };
        entries.push(json!({
            "package": artifact.package,
            "kind": kind,
            "target": artifact.name,
            "tests": tests,
        }));
    }
    let path = state.dir().join("tests.json");
    match fs::create_dir_all(state.dir())
        .and_then(|_| fs::write(&path, serde_json::to_string_pretty(&entries).unwrap_or_default()))
    {
        Ok(()) => {
            push_line(
                state,
                id,
                format!(
                    "{total} tests in {} binaries written to {}",
                    entries.len(),
                    path.display()
                ),
            )
        },
        Err(e) => push_line(state, id, format!("could not write {}: {e}", path.display())),
    }
}

fn discovered_tests(dir: &Path) -> Value {
    fs::read_to_string(dir.join("tests.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!([]))
}

// ---- files and events ------------------------------------------------------

/// Server-sent events: one line naming what changed (`pages`, `targets`,
/// `reports`, `tests`, `run`) whenever it does, with keepalives.
fn events(mut stream: TcpStream, state: &State) -> io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n",
    )?;
    stream.write_all(b": connected\n\n")?;
    stream.flush()?;
    let mut last = fingerprints(state);
    let mut ticks = 0u32;
    while !state.stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(300));
        let now = fingerprints(state);
        for (what, digest) in &now {
            if last.get(what) != Some(digest) {
                stream.write_all(format!("data: {what}\n\n").as_bytes())?;
                stream.flush()?;
            }
        }
        last = now;
        ticks += 1;
        if ticks.is_multiple_of(50) {
            stream.write_all(b": keepalive\n\n")?;
            stream.flush()?;
        }
    }
    Ok(())
}

/// A digest per watched group: `pages` (coverage/*.html), `targets`,
/// `reports`, `tests` (tests.json) and `run` (the log).
fn fingerprints(state: &State) -> BTreeMap<&'static str, u64> {
    fn stamp(hasher: &mut DefaultHasher, path: &Path) {
        if let Ok(meta) = fs::metadata(path) {
            path.hash(hasher);
            meta.len().hash(hasher);
            if let Ok(modified) = meta.modified() {
                modified.hash(hasher);
            }
        }
    }
    let dir = state.dir();
    let mut out = BTreeMap::new();
    let mut pages = DefaultHasher::new();
    for page in grid_pages(dir) {
        stamp(&mut pages, &page.path);
    }
    out.insert("pages", pages.finish());
    for (what, file) in
        [("targets", "targets.jsonl"), ("reports", "reports.jsonl"), ("tests", "tests.json")]
    {
        let mut hasher = DefaultHasher::new();
        stamp(&mut hasher, &dir.join(file));
        out.insert(what, hasher.finish());
    }
    let mut run = DefaultHasher::new();
    if let Some(log) = state.log().as_ref() {
        (log.id, log.lines.len(), log.dropped, log.finished).hash(&mut run);
    }
    out.insert("run", run.finish());
    out
}

struct Page {
    name: String,
    path: PathBuf,
    modified: u64,
}

fn grid_pages(dir: &Path) -> Vec<Page> {
    let mut pages: Vec<Page> = fs::read_dir(dir.join("coverage"))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path.file_name()?.to_str()?.strip_suffix(".html")?.to_string();
                    let modified = entry
                        .metadata()
                        .ok()?
                        .modified()
                        .ok()?
                        .duration_since(UNIX_EPOCH)
                        .ok()?
                        .as_secs();
                    Some(Page { name, path, modified })
                })
                .collect()
        })
        .unwrap_or_default();
    pages.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    pages
}

/// The latest report per property from `reports.jsonl`.
fn latest_reports(dir: &Path) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Ok(text) = fs::read_to_string(dir.join("reports.jsonl")) else {
        return out;
    };
    for line in text.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line)
            && let Some(name) = value.get("name").and_then(Value::as_str)
        {
            out.insert(name.to_string(), value);
        }
    }
    out
}

/// Everything the pages need, as JSON.
fn snapshot(state: &State) -> Value {
    let dir = state.dir();
    let reports = latest_reports(dir);
    // A page is named after a file-safe stem of the property's name
    // (`planted::bst` writes `planted_bst.html`), which its report records.
    let by_page: BTreeMap<&str, &Value> = reports
        .values()
        .filter_map(|report| {
            let page = Path::new(report.get("coverage_html")?.as_str()?);
            Some((page.file_stem()?.to_str()?, report))
        })
        .collect();
    let pages: Vec<Value> = grid_pages(dir)
        .iter()
        .map(|page| {
            json!({
                "name": page.name,
                "page": format!("/grid/{}.html", page.name),
                "modified": page.modified,
                "report": by_page.get(page.name.as_str()),
            })
        })
        .collect();
    let targets = load_targets(&dir.join("targets.jsonl")).unwrap_or_default();
    let run = state.log().as_ref().map(|log| log.json(false));
    json!({
        "dir": dir.display().to_string(),
        "pages": pages,
        "targets": targets,
        "reports": reports,
        "tests": discovered_tests(dir),
        "run": run,
        "cargo_args": state.options.cargo_args,
        "instrument": state.options.instrument,
    })
}

/// The index page: a shell that renders `/api/state` and `/api/run`.
const INDEX: &str = include_str!("serve.html");

#[cfg(test)]
mod tests {
    use super::*;

    fn request(addr: SocketAddr, raw: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    fn post(addr: SocketAddr, path: &str, body: &str, extra_headers: &str) -> (u16, String) {
        request(
            addr,
            &format!(
                "POST {path} HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}\r\n{body}",
                body.len()
            ),
        )
    }

    /// Poll `/api/run` until the run has finished; its log.
    #[cfg(unix)]
    fn wait_idle(addr: SocketAddr) -> Value {
        let mut log = Value::Null;
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(100));
            log = serde_json::from_str(&request(addr, "GET /api/run HTTP/1.1\r\n\r\n").1).unwrap();
            if log["running"] == false {
                break;
            }
        }
        assert_eq!(log["running"], false, "{log}");
        log
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ephyra-serve-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("coverage")).unwrap();
        dir
    }

    #[test]
    fn serves_pages_state_targets_and_events() {
        let dir = temp_dir("pages");
        let page = |stem: &str| dir.join("coverage").join(format!("{stem}.html"));
        fs::write(page("demo"), "<html>demo page</html>").unwrap();
        // The library names pages by a file-safe stem of the property name.
        fs::write(page("planted_bst"), "<html>bst page</html>").unwrap();
        let reports = [
            json!({
                "name": "demo",
                "status": "Passed",
                "tests": 5,
                "unique_tests": 4,
                "health": [],
                "coverage": { "functions_reached": 1, "functions_total": 2 },
                "coverage_html": page("demo"),
            }),
            json!({ "name": "planted::bst", "status": "Failed", "coverage_html": page("planted_bst") }),
        ];
        fs::write(dir.join("reports.jsonl"), reports.map(|r| format!("{r}\n")).concat()).unwrap();
        let server = start(Options { dir: dir.clone(), port: 0, ..Options::default() }).unwrap();
        let addr = server.addr();

        let (status, body) = request(addr, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(status, 200);
        assert!(body.contains("/api/state"), "{body}");

        let (status, body) = request(addr, "GET /grid/demo.html HTTP/1.1\r\n\r\n");
        assert_eq!((status, body.as_str()), (200, "<html>demo page</html>"));
        assert_eq!(request(addr, "GET /grid/../Cargo.toml HTTP/1.1\r\n\r\n").0, 404);
        assert_eq!(request(addr, "GET /grid/nope.html HTTP/1.1\r\n\r\n").0, 404);
        assert_eq!(request(addr, "GET /nowhere HTTP/1.1\r\n\r\n").0, 404);
        assert_eq!(request(addr, "DELETE / HTTP/1.1\r\n\r\n").0, 405);

        let body = "{\"file\":\"src/lib.rs\",\"begin_line\":3,\"end_line\":4}";
        let (status, reply) = post(addr, "/api/targets", body, "");
        assert_eq!((status, reply.as_str()), (200, "{\"targets\":1}"));
        assert_eq!(
            load_targets(&dir.join("targets.jsonl")).unwrap(),
            vec![CoverageTarget::new("src/lib.rs", 3..=4)]
        );
        let bad = "{\"file\":\"\",\"begin_line\":9,\"end_line\":4}";
        assert_eq!(post(addr, "/api/targets", bad, "").0, 400);
        assert_eq!(
            post(addr, "/api/targets", body, "Origin: https://evil.example\r\n").0,
            403,
            "cross-site POSTs are refused"
        );
        assert_eq!(post(addr, "/api/targets", body, &format!("Origin: http://{addr}\r\n")).0, 200);

        let (_, state) = request(addr, "GET /api/state HTTP/1.1\r\n\r\n");
        let state: Value = serde_json::from_str(&state).unwrap();
        assert_eq!(state["dir"], dir.display().to_string());
        assert_eq!(state["targets"][0]["file"], "src/lib.rs");
        let page_report = |name: &str| {
            let pages = state["pages"].as_array().unwrap();
            pages.iter().find(|p| p["name"] == name).unwrap()["report"].clone()
        };
        assert_eq!(page_report("demo")["tests"], 5);
        assert_eq!(page_report("planted_bst")["name"], "planted::bst");
        assert_eq!(state["reports"]["demo"]["coverage"]["functions_total"], 2);
        assert_eq!(state["tests"], json!([]));
        assert_eq!(state["run"], Value::Null);
        assert_eq!(request(addr, "POST /api/targets/clear HTTP/1.1\r\n\r\n").0, 200);
        assert_eq!(fs::read_to_string(dir.join("targets.jsonl")).unwrap(), "");

        // Events: connect, change a page, expect a `pages` event.
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(b"GET /api/events HTTP/1.1\r\n\r\n").unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("HTTP/1.1 200"), "{line}");
        thread::sleep(Duration::from_millis(400));
        fs::write(dir.join("coverage/other.html"), "<html>other</html>").unwrap();
        let mut saw = String::new();
        for _ in 0..40 {
            line.clear();
            if reader.read_line(&mut line).is_err() {
                break;
            }
            if line.starts_with("data:") {
                saw = line.trim().to_string();
                break;
            }
        }
        assert_eq!(saw, "data: pages");
        drop(reader);
        server.stop();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn targets_are_validated() {
        assert!(parse_target(b"{\"file\":\"a.rs\",\"begin_line\":1,\"end_line\":1}").is_ok());
        assert!(parse_target(b"{\"file\":\"a.rs\",\"begin_line\":0,\"end_line\":1}").is_err());
        assert!(parse_target(b"{\"file\":\"a\\nb\",\"begin_line\":1,\"end_line\":1}").is_err());
        assert!(parse_target(b"nope").is_err());
    }

    #[test]
    fn run_arguments_follow_the_spec() {
        let extra = vec!["--features".to_string(), "coverage".to_string()];
        let spec = RunSpec {
            package: Some("ephyra".into()),
            kind: "test".into(),
            target: Some("reports".into()),
            test: Some("planted::bst".into()),
        };
        assert_eq!(
            run_args(Some(&spec), &extra).unwrap(),
            [
                "test",
                "-p",
                "ephyra",
                "--tests",
                "--features",
                "coverage",
                "--",
                "planted::bst",
                "--exact"
            ]
        );
        let example = RunSpec {
            package: Some("ephyra".into()),
            kind: "example".into(),
            target: Some("bst".into()),
            test: None,
        };
        assert_eq!(
            run_args(Some(&example), &[]).unwrap(),
            ["run", "-p", "ephyra", "--example", "bst"]
        );
        assert_eq!(run_args(None, &extra).unwrap(), ["test", "--tests", "--features", "coverage"]);
        assert!(run_args(Some(&RunSpec { package: None, ..spec }), &[]).is_err());
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("plain"), "plain");
    }

    #[test]
    fn run_environment_routes_every_artifact_through_the_selected_directory() {
        let options = Options { dir: PathBuf::from("chosen-artifacts"), ..Options::default() };
        let env: BTreeMap<_, _> = run_env(&options).into_iter().collect();
        let ends_with = |name: &str, suffix: &str| {
            Path::new(env.get(name).unwrap()).ends_with(Path::new(suffix))
        };

        assert!(ends_with("EPHYRA_REPORT_JSON", "chosen-artifacts/reports.jsonl"));
        assert!(ends_with("EPHYRA_OBSERVATIONS", "chosen-artifacts/observations"));
        assert!(ends_with("EPHYRA_COVERAGE_HTML", "chosen-artifacts/coverage"));
        assert!(ends_with("EPHYRA_COVERAGE_TARGETS", "chosen-artifacts/targets.jsonl"));
        assert!(ends_with("EPHYRA_REGRESSIONS_DIR", "chosen-artifacts/regressions"));
        assert!(ends_with("EPHYRA_PROFRAW_DIR", "chosen-artifacts/profraw"));
    }

    #[test]
    fn features_follow_the_selected_package() {
        let members = vec![
            Member {
                name: "ephyra".into(),
                features: vec!["coverage".into(), "serve".into()],
                ..Member::default()
            },
            Member {
                name: "ephyra-locate".into(),
                features: vec!["tracing".into()],
                ..Member::default()
            },
        ];
        let given: Vec<String> = ["--features", "coverage", "--release"].map(String::from).to_vec();
        assert_eq!(
            cargo_args_for(Some("ephyra"), &given, &members),
            (["--release", "--features", "coverage"].map(String::from).to_vec(), vec![])
        );
        let (args, notes) = cargo_args_for(Some("ephyra-locate"), &given, &members);
        assert_eq!(args, ["--release"]);
        assert_eq!(notes, ["  dropped --features coverage: ephyra-locate has no such feature"]);
        // The whole workspace: any member with the feature keeps it.
        assert_eq!(
            cargo_args_for(None, &given, &members).0,
            ["--release", "--features", "coverage"]
        );
        // `pkg/feature` needs that package selected.
        let scoped: Vec<String> = ["--features=ephyra/coverage,tracing"].map(String::from).to_vec();
        assert_eq!(
            cargo_args_for(Some("ephyra-locate"), &scoped, &members).0,
            ["--features", "tracing"]
        );
        assert_eq!(
            cargo_args_for(Some("ephyra"), &scoped, &members).0,
            ["--features", "ephyra/coverage"]
        );
        // Without metadata, pass everything through.
        assert_eq!(
            cargo_args_for(Some("x"), &given, &[]).0,
            ["--release", "--features", "coverage"]
        );
    }

    #[test]
    fn artifacts_and_test_lists_are_parsed() {
        let stdout = concat!(
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///w/crates/ephyra#0.2.0\",\"target\":{\"kind\":[\"test\"],\"name\":\"reports\"},\"profile\":{\"test\":true},\"executable\":\"/w/target/deps/reports-1\"}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"registry+https://x#serde@1.0.0\",\"target\":{\"kind\":[\"lib\"],\"name\":\"serde\"},\"profile\":{\"test\":false},\"executable\":null}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///w#ephyra@0.2.0\",\"target\":{\"kind\":[\"example\"],\"name\":\"bst\"},\"profile\":{\"test\":false},\"executable\":\"/w/target/examples/bst\"}\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n",
        );
        let artifacts = parse_artifacts(stdout);
        assert_eq!(artifacts.len(), 2);
        assert_eq!(
            artifacts[0],
            Artifact {
                package: "ephyra".into(),
                kind: "test".into(),
                name: "reports".into(),
                executable: PathBuf::from("/w/target/deps/reports-1"),
                test: true,
            }
        );
        assert_eq!(
            (artifacts[1].package.as_str(), artifacts[1].kind.as_str()),
            ("ephyra", "example")
        );
        assert_eq!(package_name("registry+https://x#serde@1.0.0"), "serde");
        assert_eq!(parse_test_list("a::b: test\nc: benchmark\nd: test\n"), ["a::b", "d"]);
    }

    #[cfg(unix)]
    #[test]
    fn runs_cargo_and_streams_its_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("run");
        let fake = dir.join("fake-cargo.sh");
        fs::write(
            &fake,
            "#!/bin/sh\necho \"args: $@\"\necho \"html: $EPHYRA_COVERAGE_HTML\"\necho \"flags: $RUSTFLAGS\" >&2\nsleep 0.3\nexit 3\n",
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            dir.join("reports.jsonl"),
            "{\"name\":\"planted::bst\",\"status\":\"Passed\",\"run\":{\"package\":\"ephyra\",\"kind\":\"test\",\"target\":\"reports\",\"test\":\"planted::bst\"}}\n{\"name\":\"old\",\"status\":\"Passed\"}\n",
        )
        .unwrap();
        let options = Options {
            dir: dir.clone(),
            port: 0,
            cargo: fake.clone(),
            cargo_args: vec!["--features".into(), "coverage".into()],
            ..Options::default()
        };
        let server = start(options).unwrap();
        let addr = server.addr();

        assert_eq!(post(addr, "/api/run", "{\"name\":\"old\"}", "").0, 400, "no run spec");
        assert_eq!(post(addr, "/api/run", "{\"name\":\"nope\"}", "").0, 400);
        let (status, body) = post(addr, "/api/run", "{\"name\":\"planted::bst\"}", "");
        assert_eq!((status, body.as_str()), (202, "{\"id\":1}"));
        assert_eq!(post(addr, "/api/run", "{}", "").0, 409, "one run at a time");

        let log = wait_idle(addr);
        assert_eq!(log["status"], 3);
        assert_eq!(log["name"], "planted::bst");
        let lines: Vec<&str> =
            log["lines"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(
            lines.contains(
                &"args: test -p ephyra --tests --features coverage -- planted::bst --exact"
            ),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("html: ") && l.ends_with("/coverage")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("instrument-coverage")), "{lines:?}");
        assert_eq!(lines.last().copied(), Some("finished: exit code 3"));

        // Everything, and an explicit spec.
        assert_eq!(post(addr, "/api/run", "", "").0, 202);
        assert_eq!(wait_idle(addr)["name"], "all tests");
        let (status, _) = post(
            addr,
            "/api/run",
            "{\"package\":\"p\",\"kind\":\"example\",\"target\":\"ex\"}",
            "",
        );
        assert_eq!(status, 202);
        server.stop();
        thread::sleep(Duration::from_millis(500));
        let _ = fs::remove_dir_all(&dir);
    }
}
