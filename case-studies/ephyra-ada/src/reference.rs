//! Cases run on whatwg-url, the standard's reference implementation (a
//! JavaScript transcription of its algorithms), through one long-lived
//! `node reference/run.js` process. `npm install` in `reference/` first.

use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{mpsc, Mutex},
    time::Duration,
};

use serde_json::{json, Value};

use crate::{Case, Parts, Setter, Step, Transcript};

struct Node {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

static NODE: Mutex<Option<Node>> = Mutex::new(None);

fn start() -> Node {
    let mut child = Command::new("node")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/reference/run.js"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("node on the PATH");
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    Node { child, stdin, stdout }
}

fn name(s: Setter) -> &'static str {
    match s {
        Setter::Href => "href",
        Setter::Protocol => "protocol",
        Setter::Username => "username",
        Setter::Password => "password",
        Setter::Host => "host",
        Setter::Hostname => "hostname",
        Setter::Port => "port",
        Setter::Pathname => "pathname",
        Setter::Search => "search",
        Setter::Hash => "hash",
    }
}

fn parts(v: &Value) -> Option<Parts> {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
    v.is_object().then(|| Parts {
        href: s("href"),
        origin: s("origin"),
        protocol: s("protocol"),
        username: s("username"),
        password: s("password"),
        host: s("host"),
        hostname: s("hostname"),
        port: s("port"),
        pathname: s("pathname"),
        search: s("search"),
        hash: s("hash"),
    })
}

/// The transcript of `case` in whatwg-url. Setters other than `href`
/// report nothing; they count as succeeding.
pub fn run(case: &Case) -> Transcript {
    let line = json!({
        "input": case.input,
        "base": case.base,
        "sets": case.sets.iter().map(|(s, v)| json!([name(*s), v])).collect::<Vec<_>>(),
    })
    .to_string();
    let v = ask(line, None).expect("an answer from run.js");
    Transcript {
        can_parse: v["can_parse"].as_bool().unwrap(),
        steps: v["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Step { ok: s["ok"].as_bool().unwrap(), url: parts(&s["url"]) })
            .collect(),
    }
}

/// Sends `request` to run.js and returns its answer.
pub fn request(request: &Value) -> Value {
    ask(request.to_string(), None).expect("an answer from run.js")
}

/// [`request`], or `None` when run.js takes longer than `limit`: the
/// polyfill's JS RegExp backtracks for minutes on some URLPattern regexes,
/// and a running regex cannot be interrupted, so the process is killed and
/// the next request starts a new one.
pub fn request_within(request: &Value, limit: Duration) -> Option<Value> {
    ask(request.to_string(), Some(limit))
}

fn ask(line: String, limit: Option<Duration>) -> Option<Value> {
    // Node's readline ends lines at U+2028 and U+2029 too; JSON allows them
    // escaped.
    let line = line.replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    let mut node = NODE.lock().unwrap_or_else(|e| e.into_inner());
    let n = node.get_or_insert_with(start);
    writeln!(n.stdin, "{line}").unwrap();
    let pid = n.child.id().to_string();
    let (done, wait) = mpsc::channel::<()>();
    let watchdog = limit.map(|d| {
        std::thread::spawn(move || {
            let late = wait.recv_timeout(d) == Err(mpsc::RecvTimeoutError::Timeout);
            if late {
                let _ = Command::new("kill").args(["-9", &pid]).status();
            }
            late
        })
    });
    let mut answer = String::new();
    let read = n.stdout.read_line(&mut answer);
    drop(done);
    let killed = watchdog.is_some_and(|w| w.join().unwrap());
    if killed || read.is_err() || answer.is_empty() {
        if let Some(mut old) = node.take() {
            let _ = old.child.kill();
            let _ = old.child.wait();
        }
        return None;
    }
    Some(serde_json::from_str(&answer).expect("an answer from run.js"))
}
