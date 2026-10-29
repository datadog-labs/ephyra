// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! ephyra driver for litellm: `search <build-dir> <seed> <seconds> <out-dir>`.
//!
//! Each case is one generated provider response (`Case`), sent as a JSON line
//! to a long-lived Python worker (`harness/worker.py`) that serves it through a
//! mock HTTP server, calls litellm non-streaming and streaming, and answers
//! `{"ok":true}` or `{"ok":false,"sig":..,"msg":..}`. One `ephyra::check` per
//! (provider, round), bounded in tests and time; failures are shrunk by ephyra
//! and written as self-contained findings.

use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use ephyra::{check, gens, Arbitrary, Config, Gen, GenExt};
use serde_json::{json, Value};

// ---------------------------------------------------------------- generators

/// Small alphabet: ASCII, JSON specials, multi-byte UTF-8 (2, 3, 4 bytes),
/// line separators, so fragment and TCP-write boundaries fall inside chars.
fn text_char() -> impl Gen<Value = char> {
    gens::pick_from(vec![
        'a', 'b', 'z', ' ', '\n', '"', '\\', '{', '}', ',', ':', 'é', 'ß', '中', '文', '😀', '\u{2028}', '\t', '/', '<',
    ])
}

fn piece() -> impl Gen<Value = String> {
    gens::string_len(text_char(), 0..=6)
}

fn pieces() -> impl Gen<Value = Vec<String>> {
    gens::vec(piece()).len(0..=5)
}

fn key() -> impl Gen<Value = String> {
    gens::pick_from(vec!["a", "b", "city", "q", "ü", "k y", "", "type", "id"].into_iter().map(String::from).collect())
}

#[derive(Clone, Debug, Arbitrary)]
enum Val {
    Null,
    Bool(bool),
    Int(#[ephyra(gen = gens::int(-1000i64..=1000))] i64),
    Big(#[ephyra(gen = gens::pick_from(vec![i64::MAX, i64::MIN, 9007199254740993, -1, 0]))] i64),
    Float(#[ephyra(gen = gens::pick_from(vec![0.5f64, -1.25, 1e-7, 1e21, 123456789.125, 0.1]))] f64),
    Str(#[ephyra(gen = piece())] String),
    List(#[ephyra(gen = gens::vec(piece()).len(0..=3))] Vec<String>),
    Obj(#[ephyra(gen = gens::vec((key(), piece())).len(0..=3))] Vec<(String, String)>),
}

impl Val {
    fn json(&self) -> Value {
        match self {
            Val::Null => Value::Null,
            Val::Bool(b) => json!(b),
            Val::Int(i) | Val::Big(i) => json!(i),
            Val::Float(f) => json!(f),
            Val::Str(s) => json!(s),
            Val::List(v) => json!(v),
            Val::Obj(kv) => Value::Object(kv.iter().map(|(k, v)| (k.clone(), json!(v))).collect()),
        }
    }
}

#[derive(Clone, Debug, Arbitrary)]
enum Block {
    Text(#[ephyra(gen = pieces())] Vec<String>),
    Thinking(#[ephyra(gen = pieces())] Vec<String>, #[ephyra(gen = gens::pick_from(vec![0u8, 1, 2]))] u8),
    Redacted(#[ephyra(gen = gens::pick_from(vec![0u8, 1]))] u8),
    Tool {
        #[ephyra(gen = gens::pick_from(vec![0u8, 1, 2, 3]))]
        name: u8,
        #[ephyra(gen = gens::vec((key(), ephyra::any::<Val>())).len(0..=4))]
        args: Vec<(String, Val)>,
        /// fragment sizes (chars) for the serialized arguments; 0 = empty fragment
        #[ephyra(gen = gens::vec(gens::int(0usize..=9)).len(0..=8))]
        cuts: Vec<usize>,
        /// Gemini: a thoughtSignature on the functionCall part (Gemini 3 signs the first call of a step)
        #[ephyra(gen = gens::pick_from(vec![0u8, 0, 1, 2]))]
        sig: u8,
    },
}

fn usage_n() -> impl Gen<Value = u64> {
    gens::frequency(vec![
        (4, gens::int(0u64..=50).boxed()),
        (3, gens::int(0u64..=5000).boxed()),
        (2, gens::int(0u64..=400_000).boxed()),
        (1, gens::pick_from(vec![0u64, 1, 128_000, 199_999, 200_000, 200_001, 272_000, 1_000_000]).boxed()),
    ])
}

// ---- request side: a generated conversation (oracle `req`)

fn part() -> impl Gen<Value = String> {
    gens::frequency(vec![
        (5, gens::string_len(text_char(), 1..=8).boxed()),
        (1, piece().boxed()),
        // JSON-looking text (tool results that are JSON objects take another path for Gemini)
        (1, gens::pick_from(vec![r#"{"temp": 20, "unit": "C"}"#, "[1, 2]", r#" {"a": {"b": null}} "#, r#"{"x": "中"}"#, "{", "{}", r#"{"content": "x"}"#].into_iter().map(String::from).collect()).boxed()),
    ])
}

fn image_gen() -> impl Gen<Value = Option<(u8, u8)>> {
    gens::frequency(vec![(4, gens::constant(None).boxed()), (1, (gens::int(0u8..=1), gens::int(0u8..=3)).map(Some).boxed())])
}

#[derive(Clone, Debug, Arbitrary)]
struct Txt {
    #[ephyra(gen = gens::vec(part()).len(1..=3))]
    parts: Vec<String>,
    /// content as a list of text parts instead of one string
    list: bool,
}

#[derive(Clone, Debug, Arbitrary)]
struct SysMsg {
    txt: Txt,
    developer: bool,
}

#[derive(Clone, Debug, Arbitrary)]
struct Call {
    /// tool-call id shape (see ID_STYLES); ids are unique per conversation
    #[ephyra(gen = gens::int(0u8..=5))]
    style: u8,
    #[ephyra(gen = gens::pick_from(vec![0u8, 1, 2, 3]))]
    name: u8,
    /// small: identical parallel calls (same name and arguments) must happen
    #[ephyra(gen = gens::frequency(vec![(2, gens::constant(vec![]).boxed()), (3, gens::vec((key(), ephyra::any::<Val>())).len(1..=3).boxed())]))]
    args: Vec<(String, Val)>,
    pretty: bool,
}

#[derive(Clone, Debug, Arbitrary)]
enum Turn {
    User {
        txt: Txt,
        /// (which image, position among the text parts): a base64 data-URL image
        #[ephyra(gen = image_gen())]
        image: Option<(u8, u8)>,
    },
    Assistant {
        txt: Option<Txt>,
        #[ephyra(gen = gens::vec(ephyra::any::<Call>()).len(0..=3))]
        calls: Vec<Call>,
        /// one result per call (cycled; missing ones are "ok")
        #[ephyra(gen = gens::vec(ephyra::any::<Txt>()).len(0..=3))]
        results: Vec<Txt>,
        /// tool results sent in reverse call order
        rev: bool,
        /// an image in the first tool result
        #[ephyra(gen = image_gen())]
        result_image: Option<(u8, u8)>,
    },
}

#[derive(Clone, Debug, Arbitrary)]
struct Req {
    #[ephyra(gen = gens::vec(ephyra::any::<SysMsg>()).len(0..=2))]
    system: Vec<SysMsg>,
    #[ephyra(gen = gens::vec(ephyra::any::<Turn>()).len(0..=6))]
    turns: Vec<Turn>,
    #[ephyra(gen = gens::vec(gens::pick_from(vec![0u8, 1, 2, 3])).len(0..=2))]
    tools: Vec<u8>,
    /// 0 none, 1 auto, 2 "none", 3 required, 4/5 a declared tool by name
    #[ephyra(gen = gens::int(0u8..=5))]
    choice: u8,
    #[ephyra(gen = gens::int(0u8..=4))]
    stop: u8,
    #[ephyra(gen = gens::option(gens::pick_from(vec![1u64, 16, 77, 4096, 128_000])))]
    max_tokens: Option<u64>,
    /// parallel_tool_calls (sent with tools only)
    #[ephyra(gen = gens::option(gens::bool()))]
    parallel: Option<bool>,
}

const ID_STYLES: [&str; 6] = ["call_{n}", "toolu_0{n}", "functions.{name}:{n}", "call-{n}", "{n}", "fc_68e3a9c1d2b4f5e6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0{n}"];
const STOP_SETS: [Option<&[&str]>; 5] = [None, Some(&["END"]), Some(&["a", "\n\n"]), Some(&[" "]), Some(&["}", "STOP", "中"])];

fn txt_json(t: &Txt) -> Value {
    json!({"parts": t.parts, "list": t.list})
}

impl Req {
    fn render(&self) -> Value {
        let system: Vec<Value> = self
            .system
            .iter()
            .map(|s| json!({"parts": s.txt.parts, "list": s.txt.list, "role": if s.developer { "developer" } else { "system" }}))
            .collect();
        let mut n = 0usize;
        let mut turns = Vec::new();
        for t in &self.turns {
            match t {
                Turn::User { txt: x, image } => turns.push(json!({"t": "user", "parts": x.parts, "list": x.list,
                    "image": image.map(|(i, pos)| json!({"i": i, "pos": pos}))})),
                Turn::Assistant { txt, calls, results, rev, result_image } => {
                    let calls: Vec<Value> = calls
                        .iter()
                        .map(|c| {
                            n += 1;
                            let name = TOOL_NAMES[c.name as usize];
                            let id = ID_STYLES[c.style as usize].replace("{n}", &n.to_string()).replace("{name}", name);
                            let obj = Value::Object(c.args.iter().map(|(k, v)| (k.clone(), v.json())).collect());
                            let args = if c.pretty { serde_json::to_string_pretty(&obj).unwrap() } else { obj.to_string() };
                            json!({"id": id, "name": name, "args": args})
                        })
                        .collect();
                    let mut res: Vec<Value> = (0..calls.len())
                        .map(|i| if results.is_empty() { json!({"parts": ["ok"], "list": false}) } else { txt_json(&results[i % results.len()]) })
                        .collect();
                    if let (Some(r), Some((i, pos))) = (res.first_mut(), result_image) {
                        r["image"] = json!({"i": i, "pos": pos});
                    }
                    let (parts, list) = match txt {
                        Some(x) => (x.parts.clone(), x.list),
                        None if calls.is_empty() => (vec!["ok".to_string()], false),
                        None => (vec![], false),
                    };
                    turns.push(json!({"t": "assistant", "parts": parts, "list": list, "calls": calls, "results": res, "rev": rev}));
                }
            }
        }
        // the conversation starts with a user turn and does not end on a plain assistant turn
        if turns.first().map(|t| t["t"] != "user").unwrap_or(true) {
            turns.insert(0, json!({"t": "user", "parts": ["hi"], "list": false}));
        }
        if let Some(last) = turns.last() {
            if last["t"] == "assistant" && last["calls"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
                turns.push(json!({"t": "user", "parts": ["go on"], "list": false}));
            }
        }
        let tools: Vec<&str> = self.tools.iter().map(|&i| TOOL_NAMES[i as usize]).collect();
        let mut names: Vec<&str> = tools.clone();
        for t in &turns {
            for c in t["calls"].as_array().into_iter().flatten() {
                names.push(c["name"].as_str().unwrap());
            }
        }
        let choice = match self.choice {
            0 => Value::Null,
            1 => json!("auto"),
            2 => json!("none"),
            3 => json!("required"),
            k if !names.is_empty() => json!(names[(k as usize - 4) % names.len()]),
            _ => json!("auto"),
        };
        let stop = STOP_SETS[self.stop as usize];
        json!({"system": system, "turns": turns, "tools": tools, "choice": choice, "stop": stop, "max_tokens": self.max_tokens, "parallel": self.parallel})
    }
}

/// An optional param of the `param` oracle: absent (mostly) or an index into its value list
fn opt_idx(n: u8) -> impl Gen<Value = Option<u8>> {
    gens::frequency(vec![(4, gens::constant(None).boxed()), (1, gens::int(0u8..=n - 1).map(Some).boxed())])
}

/// `error` oracle: the provider answers with an error (HTTP status, or an error event mid-stream)
#[derive(Clone, Debug, Arbitrary)]
struct ErrSpec {
    /// index into ERR_STATUS
    #[ephyra(gen = gens::int(0u8..=11))]
    status: u8,
    /// HTTP error, or (true) the stream starts with 200 and an error event follows after `after` events
    #[ephyra(gen = gens::frequency(vec![(2, gens::constant(false).boxed()), (1, gens::constant(true).boxed())]))]
    midstream: bool,
    #[ephyra(gen = gens::int(0u8..=6))]
    after: u8,
    /// index into ERR_MSGS
    #[ephyra(gen = gens::int(0u8..=4))]
    msg: u8,
    /// HTTP error body: 0 the provider's documented error JSON, 1 plain text, 2 empty
    #[ephyra(gen = gens::pick_from(vec![0u8, 0, 0, 1, 2]))]
    body: u8,
}

const ERR_STATUS: [u16; 12] = [400, 401, 403, 404, 408, 422, 429, 500, 502, 503, 504, 529];
/// neutral texts: none of the words litellm's exception mapping matches on (context, rate, quota, ...)
const ERR_MSGS: [&str; 5] = ["something went wrong", "request rejected (code E1234)", "unexpected condition: \u{fc}\u{2192}\u{2713}",
                              "quote \" and \\ backslash", "Upstream said no."];

impl ErrSpec {
    fn render(&self) -> Value {
        json!({"status": ERR_STATUS[self.status as usize], "midstream": self.midstream, "after": self.after,
               "msg": ERR_MSGS[self.msg as usize], "body": (["json", "text", "empty"][self.body as usize])})
    }
}

/// `router` oracle: a Router over mock deployments whose answers follow a script
#[derive(Clone, Debug, Arbitrary)]
struct RouterSpec {
    /// the primary group's answers in request order (0 = 200, else 1 + index into ERR_STATUS), then 200
    #[ephyra(gen = gens::vec(gens::frequency(vec![(1, gens::constant(0u8).boxed()), (4, gens::int(1u8..=12).boxed())])).len(0..=4))]
    script: Vec<u8>,
    /// the backup group's answer to every request (0 = 200)
    #[ephyra(gen = gens::frequency(vec![(3, gens::constant(0u8).boxed()), (1, gens::int(1u8..=12).boxed())]))]
    backup: u8,
    #[ephyra(gen = gens::int(0u8..=3))]
    retries: u8,
    #[ephyra(gen = gens::int(1u8..=2))]
    deployments: u8,
    fallback: bool,
    /// RetryPolicy fields: x / 4 indexes POLICY_FIELDS, x % 4 is its retries
    #[ephyra(gen = gens::frequency(vec![(2, gens::constant(Vec::new()).boxed()),
        (1, gens::vec(gens::int(0u8..=31)).len(1..=3).boxed())]))]
    policy: Vec<u8>,
    /// error body: 0 the provider's documented error JSON, 1 plain text, 2 empty
    #[ephyra(gen = gens::pick_from(vec![0u8, 0, 0, 1, 2]))]
    body: u8,
    /// chat cases: the Router call streams (stream_chunk_builder of its chunks vs the direct stream's)
    #[ephyra(gen = gens::frequency(vec![(2, gens::constant(false).boxed()), (1, gens::constant(true).boxed())]))]
    stream: bool,
}

const POLICY_FIELDS: [&str; 8] = ["BadRequestErrorRetries", "AuthenticationErrorRetries", "TimeoutErrorRetries", "RateLimitErrorRetries",
                                  "InternalServerErrorRetries", "ServiceUnavailableErrorRetries", "NotFoundErrorRetries", "DefaultRetries"];

impl RouterSpec {
    fn render(&self) -> Value {
        let st = |x: u8| if x == 0 { Value::Null } else { json!(ERR_STATUS[(x - 1) as usize]) };
        let policy: serde_json::Map<String, Value> = self.policy.iter().map(|x| (POLICY_FIELDS[(x / 4) as usize].to_string(), json!(x % 4))).collect();
        json!({"script": self.script.iter().map(|x| st(*x)).collect::<Vec<_>>(), "backup": st(self.backup),
               "retries": self.retries, "deployments": self.deployments, "fallback": self.fallback, "policy": policy,
               "body": (["json", "text", "empty"][self.body as usize]), "stream": self.stream})
    }
}

/// Optional OpenAI chat params (`param` oracle): values include 0 / false (falsy but meaningful) and edges
#[derive(Clone, Debug, Arbitrary)]
struct Params {
    #[ephyra(gen = opt_idx(5))]
    temperature: Option<u8>,
    #[ephyra(gen = opt_idx(4))]
    top_p: Option<u8>,
    #[ephyra(gen = opt_idx(3))]
    seed: Option<u8>,
    #[ephyra(gen = opt_idx(4))]
    presence_penalty: Option<u8>,
    #[ephyra(gen = opt_idx(3))]
    frequency_penalty: Option<u8>,
    #[ephyra(gen = opt_idx(3))]
    n: Option<u8>,
    #[ephyra(gen = opt_idx(2))]
    logprobs: Option<u8>,
    #[ephyra(gen = opt_idx(3))]
    top_logprobs: Option<u8>,
    #[ephyra(gen = opt_idx(3))]
    user: Option<u8>,
    #[ephyra(gen = opt_idx(6))]
    reasoning_effort: Option<u8>,
    #[ephyra(gen = opt_idx(4))]
    max_completion_tokens: Option<u8>,
    #[ephyra(gen = opt_idx(2))]
    response_format: Option<u8>,
    drop: bool,
}

impl Params {
    fn render(&self) -> (Value, bool) {
        let mut m = serde_json::Map::new();
        let mut put = |k: &str, i: Option<u8>, vals: Vec<Value>| {
            if let Some(i) = i {
                m.insert(k.to_string(), vals[i as usize].clone());
            }
        };
        put("temperature", self.temperature, vec![json!(0.0), json!(0.37), json!(1), json!(1.7), json!(2.0)]);
        put("top_p", self.top_p, vec![json!(0.0), json!(0.13), json!(0.9), json!(1)]);
        put("seed", self.seed, vec![json!(0), json!(7), json!(123457)]);
        put("presence_penalty", self.presence_penalty, vec![json!(0.0), json!(0.25), json!(-1.5), json!(2.0)]);
        put("frequency_penalty", self.frequency_penalty, vec![json!(0.0), json!(0.41), json!(-2.0)]);
        put("n", self.n, vec![json!(1), json!(2), json!(4)]);
        put("logprobs", self.logprobs, vec![json!(true), json!(false)]);
        put("top_logprobs", self.top_logprobs, vec![json!(0), json!(3), json!(20)]);
        put("user", self.user, vec![json!("u-1"), json!("user@example.com"), json!("ユーザー 1")]);
        put("reasoning_effort", self.reasoning_effort, vec![json!("none"), json!("minimal"), json!("low"), json!("medium"), json!("high"), json!("xhigh")]);
        put("max_completion_tokens", self.max_completion_tokens, vec![json!(1), json!(64), json!(4096), json!(32000)]);
        put("response_format", self.response_format, vec![json!("json_object"), json!("json_schema")]);
        (Value::Object(m), self.drop)
    }
}

#[derive(Clone, Debug, Arbitrary)]
struct Case {
    /// the request: None = one user message "hi" (the response-side oracles alone)
    #[ephyra(gen = gens::option(ephyra::any::<Req>()))]
    req: Option<Req>,
    #[ephyra(gen = gens::vec(ephyra::any::<Block>()).len(0..=6))]
    blocks: Vec<Block>,
    #[ephyra(gen = gens::int(0u8..=3))]
    stop: u8,
    #[ephyra(gen = gens::int(0u8..=2))]
    model: u8,
    #[ephyra(gen = usage_n())]
    input: u64,
    #[ephyra(gen = usage_n())]
    output: u64,
    #[ephyra(gen = gens::option(usage_n()))]
    cache_read: Option<u64>,
    #[ephyra(gen = gens::option(usage_n()))]
    cache_creation: Option<u64>,
    #[ephyra(gen = gens::option(usage_n()))]
    reasoning: Option<u64>,
    /// TCP write sizes, cycled; empty = one write
    #[ephyra(gen = gens::vec(gens::frequency(vec![(3, gens::int(1usize..=3).boxed()), (2, gens::int(1usize..=200).boxed())])).len(0..=4))]
    writes: Vec<usize>,
    ascii: bool,
    delta_usage_full: bool,
    is_async: bool,
    /// 0: chat only; 1: also litellm.responses(); 2: also litellm.anthropic.messages (same payload)
    #[ephyra(gen = gens::pick_from(vec![0u8, 0, 1, 2]))]
    bridge: u8,
    /// response_format (bedrock only): 0 none, 1 json_object, 2 json_schema
    #[ephyra(gen = gens::pick_from(vec![0u8, 0, 0, 0, 1, 2, 2]))]
    rf: u8,
    /// `turn` oracle: how the response message goes back as history (0 dict without None fields,
    /// 1 the Message object, 2 model_dump() with every field)
    #[ephyra(gen = gens::int(0u8..=2))]
    turn: u8,
    /// `cache` oracle: 0 off; with the in-memory response cache on, the first call is 1 non-stream or
    /// 2 stream, then the same call is repeated (non-stream and stream replay) and must be a cache hit
    #[ephyra(gen = gens::pick_from(vec![0u8, 0, 0, 1, 2]))]
    cache: u8,
    /// `param` oracle: optional chat params added to one more call (on about a third of the cases)
    #[ephyra(gen = gens::frequency(vec![(2, gens::constant(None).boxed()), (1, ephyra::any::<Params>().map(Some).boxed())]))]
    params: Option<Params>,
    /// `error` oracle (on about a quarter of the cases): the same calls against a provider error
    #[ephyra(gen = gens::frequency(vec![(3, gens::constant(None).boxed()), (1, ephyra::any::<ErrSpec>().map(Some).boxed())]))]
    err: Option<ErrSpec>,
    /// `router` oracle (on about a fifth of the cases): retries / fallbacks of a Router over scripted errors
    #[ephyra(gen = gens::frequency(vec![(4, gens::constant(None).boxed()), (1, ephyra::any::<RouterSpec>().map(Some).boxed())]))]
    rt: Option<RouterSpec>,
}

const STOPS: [&str; 4] = ["end_turn", "stop_sequence", "max_tokens", "tool_use"];
const TOOL_NAMES: [&str; 4] = ["get_weather", "search", "f", "do_thing"];

/// Signatures and redacted data are unique per block in real responses (litellm dedups replayed thinking
/// by signature); a 4-char base64 prefix keeps the value valid base64 (Claude's redacted data always is).
fn uniq(i: usize, s: &str) -> String {
    format!("{}AAA{s}", (b'A' + i as u8) as char)
}

fn cut(s: &str, cuts: &[usize]) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut pos = 0;
    for &n in cuts {
        if pos >= chars.len() {
            break;
        }
        let end = (pos + n).min(chars.len());
        out.push(chars[pos..end].iter().collect());
        pos = end;
    }
    if pos < chars.len() {
        out.push(chars[pos..].iter().collect());
    }
    out
}

impl Case {
    fn render(&self, provider: &str) -> Value {
        let anthropic = provider == "anthropic";
        let gemini = provider == "gemini";
        let bedrock = provider == "bedrock";
        let models: [&str; 3] = match provider {
            "anthropic" => ["claude-sonnet-4-5", "claude-haiku-4-5", "claude-opus-4-6"],
            "gemini" => ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-3.1-pro-preview"],
            "bedrock" => ["us.anthropic.claude-sonnet-4-5-20250929-v1:0", "amazon.nova-pro-v1:0", "us.anthropic.claude-opus-4-6-v1"],
            _ => ["gpt-4o", "o3-mini", "gpt-5"],
        };
        let mut blocks = Vec::new();
        for (i, b) in self.blocks.iter().enumerate() {
            match b {
                Block::Text(p) => blocks.push(json!({"t": "text", "pieces": p})),
                Block::Thinking(p, s) if gemini => {
                    blocks.push(json!({"t": "thinking", "pieces": p, "sig": ([None, Some("c2lnbmF0dXJl"), Some("Zm9vYmFy+/==")][*s as usize]).map(|x| uniq(i, x))}))
                }
                Block::Thinking(p, s) if anthropic || bedrock => {
                    blocks.push(json!({"t": "thinking", "pieces": p, "sig": uniq(i, ["EqMBsig0", "c2lnbmF0dXJl", "Zm9vYmFy+/=="][*s as usize])}))
                }
                Block::Redacted(d) if anthropic || bedrock => {
                    blocks.push(json!({"t": "redacted", "data": uniq(i, ["EmwKAhgBEgy3va3pzix/LafPsn4a", "EmwKAhgB"][*d as usize])}))
                }
                Block::Tool { name, args, cuts, sig } => {
                    let obj: serde_json::Map<String, Value> = args.iter().map(|(k, v)| (k.clone(), v.json())).collect();
                    let text = if obj.is_empty() && cuts.is_empty() { String::new() } else { Value::Object(obj).to_string() };
                    let id = if anthropic { format!("toolu_{i:02}") } else if bedrock { format!("tooluse_{i:02}") } else { format!("call_{i:02}") };
                    let sig = if gemini { [None, Some("dG9vbHNpZw=="), Some("EpYCCpMCAb4+9vu/x0==")][*sig as usize].map(|x| uniq(i, x)) } else { None };
                    blocks.push(json!({"t": "tool", "id": id, "name": TOOL_NAMES[*name as usize], "pieces": cut(&text, cuts), "sig": sig}));
                }
                _ => {}
            }
        }
        // OpenAI documents cached_tokens as a subset of prompt_tokens
        let cache_read = if anthropic || bedrock { self.cache_read } else { self.cache_read.map(|c| c.min(self.input)) };
        let reasoning = if anthropic || bedrock { None } else if gemini { self.reasoning } else { self.reasoning.map(|r| r.min(self.output)) };
        json!({
            "provider": provider,
            "model": models[self.model as usize],
            "blocks": blocks,
            "stop": STOPS[self.stop as usize],
            "usage": {"input": self.input, "output": self.output, "cache_read": cache_read,
                      "cache_creation": if anthropic || bedrock { self.cache_creation } else { None }, "reasoning": reasoning,
                      // Bedrock cacheDetails: the 1h part of the cache write (the rest is 5m)
                      "cache_1h": if bedrock { self.reasoning.zip(self.cache_creation).map(|(r, w)| r.min(w)) } else { None }},
            "writes": self.writes,
            "ascii": self.ascii,
            "delta_usage_full": self.delta_usage_full,
            "async": self.is_async,
            "include_usage": true,
            "bridge": (["none", "responses", "messages"][self.bridge as usize]),
            "rf": if bedrock { ["none", "json_object", "json_schema"][self.rf as usize] } else { "none" },
            "req": self.req.as_ref().map(|r| r.render()),
            "turn": (["dict", "object", "dump"][self.turn as usize]),
            "cache": (["off", "nonstream", "stream"][self.cache as usize]),
            "params": self.params.as_ref().map(|p| p.render().0),
            "drop_params": self.params.as_ref().map(|p| p.render().1).unwrap_or(false),
            "err": self.err.as_ref().map(|e| e.render()),
            "rt": self.rt.as_ref().map(|r| r.render()),
        })
    }
}

// ---------------------------------------------------------------- worker

struct Worker {
    build: PathBuf,
    log: PathBuf,
    child: Option<(Child, ChildStdin, BufReader<ChildStdout>)>,
}

enum Answer {
    Ok,
    Fail(String, String),
    Hang,
    Died,
}

impl Worker {
    fn start(&mut self) {
        let py = std::env::var("LITELLM_PY").expect("source env.sh first");
        let worker = self.build.join("harness/worker.py");
        let mut cmd = Command::new(&py);
        if std::env::var("COVERAGE_FILE").is_ok() && std::env::var("COVERAGE_MODE").as_deref() == Ok("coverage") {
            let src = std::env::var("LITELLM_SRC").unwrap();
            cmd.args(["-m", "coverage", "run", "-p", &format!("--source={src}/litellm")]);
        }
        let mut child = cmd
            .arg(&worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(fs::OpenOptions::new().create(true).append(true).open(&self.log).expect("worker log")))
            .spawn()
            .expect("start worker");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        self.child = Some((child, stdin, stdout));
        match self.ask_raw("{\"ping\":1}", Duration::from_secs(180)) {
            Some(_) => {}
            None => panic!("worker did not start"),
        }
    }

    fn ask_raw(&mut self, line: &str, limit: Duration) -> Option<String> {
        let (child, stdin, stdout) = self.child.as_mut().unwrap();
        if writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_err() {
            return None;
        }
        let pid = child.id().to_string();
        let (done, wait) = mpsc::channel::<()>();
        let watchdog = std::thread::spawn(move || {
            let late = wait.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout);
            if late {
                let _ = Command::new("kill").args(["-9", &pid]).status();
            }
            late
        });
        let mut answer = String::new();
        let read = stdout.read_line(&mut answer);
        drop(done);
        let late = watchdog.join().unwrap();
        if late || read.is_err() || answer.is_empty() {
            return None;
        }
        Some(answer)
    }

    fn ask(&mut self, case: &Value) -> Answer {
        if self.child.is_none() {
            self.start();
        }
        let t = Instant::now();
        let line = json!({"case": case}).to_string();
        match self.ask_raw(&line, Duration::from_secs(60)) {
            Some(a) => {
                let v: Value = serde_json::from_str(&a).expect("worker answer is JSON");
                if v["ok"].as_bool() == Some(true) {
                    Answer::Ok
                } else {
                    Answer::Fail(v["sig"].as_str().unwrap_or("harness__error__noanswer").into(), v["msg"].as_str().unwrap_or("").into())
                }
            }
            None => {
                self.kill();
                if t.elapsed() >= Duration::from_secs(59) { Answer::Hang } else { Answer::Died }
            }
        }
    }

    fn kill(&mut self) {
        if let Some((mut c, _, _)) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Clean exit so coverage.py writes its data.
    fn quit(&mut self) {
        if let Some((mut c, mut stdin, _)) = self.child.take() {
            let _ = writeln!(stdin, "{{\"quit\":1}}");
            drop(stdin);
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(30) {
                if let Ok(Some(_)) = c.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ---------------------------------------------------------------- failures

/// One panic site per oracle kind, so shrinking keeps the failure it started with.
fn fail(sig: &str, msg: &str) -> ! {
    let kind: Vec<&str> = sig.splitn(3, "__").collect();
    let k = (kind.first().copied().unwrap_or(""), kind.get(1).copied().unwrap_or(""));
    match k {
        ("crash", "exception") => panic!("{sig}: {msg}"),
        ("crash", _) => panic!("{sig}: {msg}"),
        ("hang", _) => panic!("{sig}: {msg}"),
        ("model", "content") => panic!("{sig}: {msg}"),
        ("model", "tool_calls") => panic!("{sig}: {msg}"),
        ("model", _) => panic!("{sig}: {msg}"),
        ("stream", "content") => panic!("{sig}: {msg}"),
        ("stream", "reasoning") => panic!("{sig}: {msg}"),
        ("stream", "thinking") => panic!("{sig}: {msg}"),
        ("stream", "tools") => panic!("{sig}: {msg}"),
        ("stream", "finish") => panic!("{sig}: {msg}"),
        ("stream", "usage") => panic!("{sig}: {msg}"),
        ("stream", "cost") => panic!("{sig}: {msg}"),
        ("stream", _) => panic!("{sig}: {msg}"),
        ("resp", k) if k.starts_with("model") => panic!("{sig}: {msg}"),
        ("resp", "sequence_number") => panic!("{sig}: {msg}"),
        ("resp", _) => panic!("{sig}: {msg}"),
        ("msgs", k) if k.starts_with("model") => panic!("{sig}: {msg}"),
        ("msgs", "thinking_signature") => panic!("{sig}: {msg}"),
        ("msgs", _) => panic!("{sig}: {msg}"),
        ("req", k) if k.starts_with("messages") => panic!("{sig}: {msg}"),
        ("req", "stream_diff") => panic!("{sig}: {msg}"),
        ("req", "system") => panic!("{sig}: {msg}"),
        ("req", "tools") => panic!("{sig}: {msg}"),
        ("req", "tool_choice") => panic!("{sig}: {msg}"),
        ("req", "parallel") => panic!("{sig}: {msg}"),
        ("req", _) => panic!("{sig}: {msg}"),
        ("cost", _) => panic!("{sig}: {msg}"),
        ("cache", "miss") => panic!("{sig}: {msg}"),
        ("cache", "wrong_hit") => panic!("{sig}: {msg}"),
        ("cache", _) => panic!("{sig}: {msg}"),
        ("error", "class") => panic!("{sig}: {msg}"),
        ("error", "swallowed") => panic!("{sig}: {msg}"),
        ("error", "not_raised") => panic!("{sig}: {msg}"),
        ("error", _) => panic!("{sig}: {msg}"),
        ("router", "single_attempt") => panic!("{sig}: {msg}"),
        ("router", "retries") => panic!("{sig}: {msg}"),
        ("router", "fallback") => panic!("{sig}: {msg}"),
        ("router", "result") => panic!("{sig}: {msg}"),
        ("router", "exception") => panic!("{sig}: {msg}"),
        ("router", "unmapped") => panic!("{sig}: {msg}"),
        ("router", _) => panic!("{sig}: {msg}"),
        _ => panic!("{sig}: {msg}"),
    }
}

struct Stats {
    cases: u64,
    failures: u64,
    per_sig: HashMap<String, u32>,
    harness_errors: u64,
    /// cases that failed with a signature already reported by this process (passed on, not re-reported)
    known_skips: HashMap<String, u64>,
    masks: Value,
}

fn write_stats(out: &Path, s: &Stats) {
    let v = json!({"cases": s.cases, "failures": s.failures, "signatures": s.per_sig, "harness_errors": s.harness_errors,
                    "known_sig_skips": s.known_skips, "masks": s.masks});
    let tmp = out.join("stats.json.tmp");
    fs::write(&tmp, v.to_string()).unwrap();
    fs::rename(tmp, out.join("stats.json")).unwrap();
}

const REPRO: &str = r#"#!/usr/bin/env bash
# repro.sh <build-dir>: exit 1 if the failure in case.json reproduces, 0 if not.
B=$(cd "$1" && pwd) || exit 2
cd "$(dirname "$0")" || exit 2
source "$B/env.sh" || exit 2
"$LITELLM_PY" worker.py --repro case.json "$(cat signature)"
"#;

fn write_finding(out: &Path, stats: &mut Stats, sig: &str, msg: &str, case: &Value, seed: u64, build: &Path) {
    let n = stats.per_sig.entry(sig.to_string()).or_insert(0);
    *n += 1;
    if *n > 3 {
        return;
    }
    let dir = out.join("findings").join(sig).join(n.to_string());
    fs::create_dir_all(&dir).unwrap();
    let oracle = sig.split("__").next().unwrap_or("");
    fs::write(dir.join("finding.json"), json!({"signature": sig, "oracle": oracle, "message": msg, "seed": seed}).to_string()).unwrap();
    fs::write(dir.join("case.json"), serde_json::to_string_pretty(case).unwrap()).unwrap();
    fs::write(dir.join("signature"), sig).unwrap();
    let _ = fs::copy(build.join("harness/worker.py"), dir.join("worker.py"));
    fs::write(dir.join("repro.sh"), REPRO).unwrap();
    let _ = Command::new("chmod").arg("+x").arg(dir.join("repro.sh")).status();
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        eprintln!("usage: search <build-dir> <seed> <seconds> <out-dir>");
        std::process::exit(2);
    }
    let build = fs::canonicalize(&a[1]).unwrap();
    let seed: u64 = a[2].parse().unwrap();
    let secs: u64 = a[3].parse().unwrap();
    let out = PathBuf::from(&a[4]);
    fs::create_dir_all(out.join("findings")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut stats = Stats { cases: 0, failures: 0, per_sig: HashMap::new(), harness_errors: 0, known_skips: HashMap::new(), masks: json!({}) };
    write_stats(&out, &stats);

    let mut worker = Worker { build: build.clone(), log: out.join("worker-stderr.log"), child: None };
    worker.start();
    let providers = ["anthropic", "openai", "gemini", "bedrock"];
    let mut round: u64 = 0;
    // keep some time for shrinking and a clean worker exit
    while Instant::now() + Duration::from_secs(20) < deadline {
        let provider = providers[(round % providers.len() as u64) as usize];
        let rseed = seed.wrapping_mul(1_000_003).wrapping_add(round);
        round += 1;
        let left = deadline.saturating_duration_since(Instant::now()).saturating_sub(Duration::from_secs(15));
        let mut config = Config::default()
            .tests(200)
            .seed(rseed)
            .budget(left.min(Duration::from_secs(30)))
            .name(format!("litellm_{provider}"))
            .no_regressions();
        config.shrink.max_steps = 300;
        config.shrink.timeout_ms = Some(left.min(Duration::from_secs(40)).as_millis() as u64);
        let mut first: Option<String> = None;
        let mut minimal: Option<(String, Value)> = None;
        let mut calls: u64 = 0;
        let report = check(&config, ephyra::any::<Case>(), |c: Case| {
            calls += 1;
            let case = c.render(provider);
            let (sig, msg) = match worker.ask(&case) {
                Answer::Ok => return,
                Answer::Fail(s, m) => (s, m),
                Answer::Hang => (format!("hang__timeout__{provider}"), "no answer within 60 s".into()),
                Answer::Died => (format!("crash__worker_died__{provider}"), "worker process died".into()),
            };
            // A signature this process already reported is passed on so the search
            // keeps looking for other bugs (the controller caps findings per signature anyway).
            if first.is_none() && stats.per_sig.contains_key(&sig) {
                *stats.known_skips.entry(sig).or_insert(0) += 1;
                return;
            }
            if first.is_none() {
                first = Some(sig.clone());
            }
            if first.as_deref() == Some(sig.as_str()) {
                minimal = Some((msg.clone(), case));
            }
            fail(&sig, &msg);
        });
        stats.cases += report.tests;
        if let (Some(sig), Some((msg, case))) = (first, minimal) {
            if !report.passed() {
                if sig.starts_with("harness__") {
                    stats.harness_errors += 1;
                    eprintln!("harness error: {sig}: {msg}");
                }
                stats.failures += 1;
                stats.cases += 1; // the failing case itself
                eprintln!("FAIL {sig} (round {round}, {calls} calls)");
                write_finding(&out, &mut stats, &sig, &msg, &case, rseed, &build);
            }
        }
        if let Some(a) = worker.child.is_some().then(|| worker.ask_raw("{\"counts\":1}", Duration::from_secs(30))).flatten() {
            if let Ok(v) = serde_json::from_str::<Value>(&a) {
                stats.masks = v["counts"].clone();
            }
        }
        write_stats(&out, &stats);
    }
    worker.quit();
    write_stats(&out, &stats);
}
