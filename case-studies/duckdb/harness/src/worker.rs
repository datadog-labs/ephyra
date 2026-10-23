// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The worker: a child process that owns one DuckDB database (C API, libduckdb.so of the build under
//! test) and answers one JSON request per line on stdin with one JSON line on stdout. Running it as a
//! child keeps segfaults/aborts/hangs out of the search process; the parent restarts it.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::Duration;

#[repr(C)]
struct DResult {
    cc: u64,
    rc: u64,
    rch: u64,
    cols: *mut c_void,
    err: *mut c_char,
    internal: *mut c_void,
}

extern "C" {
    fn duckdb_open(path: *const c_char, out: *mut *mut c_void) -> c_int;
    fn duckdb_close(db: *mut *mut c_void);
    fn duckdb_connect(db: *mut c_void, out: *mut *mut c_void) -> c_int;
    fn duckdb_disconnect(con: *mut *mut c_void);
    fn duckdb_query(con: *mut c_void, q: *const c_char, r: *mut DResult) -> c_int;
    fn duckdb_destroy_result(r: *mut DResult);
    fn duckdb_result_error(r: *mut DResult) -> *const c_char;
    fn duckdb_result_error_type(r: *mut DResult) -> c_int;
    fn duckdb_column_count(r: *mut DResult) -> u64;
    fn duckdb_row_count(r: *mut DResult) -> u64;
    fn duckdb_column_name(r: *mut DResult, col: u64) -> *const c_char;
    fn duckdb_column_type(r: *mut DResult, col: u64) -> c_int;
    fn duckdb_value_varchar(r: *mut DResult, col: u64, row: u64) -> *mut c_char;
    fn duckdb_value_is_null(r: *mut DResult, col: u64, row: u64) -> bool;
    fn duckdb_free(p: *mut c_void);
}

pub const ERROR_FATAL: i32 = 30;
pub const ERROR_INTERNAL: i32 = 31;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Req {
    /// open a fresh in-memory database (closing the old one)
    pub open: bool,
    pub sql: String,
    /// return rows only when the result has at most this many (hashes are always returned)
    pub cap: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Resp {
    pub ok: bool,
    pub etype: i32,
    pub msg: String,
    pub names: Vec<String>,
    pub types: Vec<i32>,
    pub nrows: u64,
    pub rows: Option<Vec<Vec<Option<String>>>>,
    /// order-independent hash of the rows (wrapping sum of row hashes)
    pub mhash: u64,
    /// order-dependent hash of the rows
    pub ohash: u64,
}

fn row_hash(row: &[Option<String>]) -> u64 {
    let mut h = DefaultHasher::new(); // SipHash with fixed zero keys: deterministic across processes
    row.hash(&mut h);
    h.finish()
}

struct Db {
    db: *mut c_void,
    con: *mut c_void,
}

impl Db {
    fn open() -> Db {
        let mut db = std::ptr::null_mut();
        let mut con = std::ptr::null_mut();
        unsafe {
            assert_eq!(duckdb_open(std::ptr::null(), &mut db), 0, "duckdb_open failed");
            assert_eq!(duckdb_connect(db, &mut con), 0, "duckdb_connect failed");
        }
        Db { db, con }
    }
    fn close(&mut self) {
        unsafe {
            duckdb_disconnect(&mut self.con);
            duckdb_close(&mut self.db);
        }
    }
    fn query(&self, sql: &str, cap: u64) -> Resp {
        let q = CString::new(sql.replace('\0', " ")).unwrap();
        let mut r = DResult { cc: 0, rc: 0, rch: 0, cols: std::ptr::null_mut(), err: std::ptr::null_mut(), internal: std::ptr::null_mut() };
        let mut out = Resp::default();
        unsafe {
            let st = duckdb_query(self.con, q.as_ptr(), &mut r);
            if st != 0 {
                out.ok = false;
                out.etype = duckdb_result_error_type(&mut r);
                let e = duckdb_result_error(&mut r);
                if !e.is_null() {
                    out.msg = CStr::from_ptr(e).to_string_lossy().into_owned();
                }
            } else {
                out.ok = true;
                let nc = duckdb_column_count(&mut r);
                let nr = duckdb_row_count(&mut r);
                out.nrows = nr;
                for c in 0..nc {
                    out.names.push(CStr::from_ptr(duckdb_column_name(&mut r, c)).to_string_lossy().into_owned());
                    out.types.push(duckdb_column_type(&mut r, c));
                }
                let mut rows = Vec::new();
                let mut oh = DefaultHasher::new();
                for i in 0..nr {
                    let mut row = Vec::with_capacity(nc as usize);
                    for c in 0..nc {
                        if duckdb_value_is_null(&mut r, c, i) {
                            row.push(None);
                        } else {
                            let p = duckdb_value_varchar(&mut r, c, i);
                            if p.is_null() {
                                row.push(Some("<unprintable>".into()));
                            } else {
                                row.push(Some(CStr::from_ptr(p).to_string_lossy().into_owned()));
                                duckdb_free(p as *mut c_void);
                            }
                        }
                    }
                    let h = row_hash(&row);
                    out.mhash = out.mhash.wrapping_add(h);
                    h.hash(&mut oh);
                    if nr <= cap {
                        rows.push(row);
                    }
                }
                out.ohash = oh.finish();
                if nr <= cap {
                    out.rows = Some(rows);
                }
            }
            duckdb_destroy_result(&mut r);
        }
        out
    }
}

/// `duckharness worker`: the child's main loop.
pub fn worker_main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut db = Db::open();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let req: Req = serde_json::from_str(&line).expect("bad request");
        if req.open {
            db.close();
            db = Db::open();
        }
        let resp = if req.sql.is_empty() { Resp { ok: true, ..Default::default() } } else { db.query(&req.sql, req.cap) };
        serde_json::to_writer(&mut stdout, &resp).unwrap();
        stdout.write_all(b"\n").unwrap();
        stdout.flush().unwrap();
    }
}

pub enum Outcome {
    Done(Resp),
    /// the worker died (signal or exit); the string describes how
    Died(String),
    Timeout,
}

/// Parent-side handle on a worker child process.
pub struct Worker {
    /// the next case must open a fresh database
    pub dirty: bool,
    /// statements that undo the debug settings of the last case (run before the next case on the same database)
    pub reset: Vec<String>,
    child: Option<(Child, ChildStdin, Receiver<String>, std::thread::JoinHandle<String>)>,
}

impl Worker {
    pub fn new() -> Worker {
        Worker { dirty: true, reset: vec![], child: None }
    }
    fn start(&mut self) {
        self.dirty = true;
        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(exe)
            .arg("worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("cannot start worker");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut s = String::new();
                match r.read_line(&mut s) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(s).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        // keep the tail of stderr (assertion text on abort)
        let errt = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            let s = String::from_utf8_lossy(&buf).into_owned();
            let n = s.len();
            s[n.saturating_sub(2000)..].to_string()
        });
        self.child = Some((child, stdin, rx, errt));
    }
    pub fn kill(&mut self) -> String {
        self.stop(false)
    }
    /// `graceful`: close stdin and give the worker time to exit by itself, so an instrumented
    /// libduckdb.so writes its LLVM profile (a killed process writes none).
    fn stop(&mut self, graceful: bool) -> String {
        if let Some((mut c, stdin, _rx, errt)) = self.child.take() {
            drop(stdin);
            if graceful {
                let t = std::time::Instant::now();
                while t.elapsed() < Duration::from_secs(10) {
                    if let Ok(Some(_)) = c.try_wait() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let _ = c.kill();
            let st = c.wait();
            let err = errt.join().unwrap_or_default();
            return format!("{:?}; stderr tail: {}", st, err);
        }
        String::new()
    }
    pub fn request(&mut self, req: &Req, limit: Duration) -> Outcome {
        if self.child.is_none() {
            self.start();
        }
        let (_, stdin, rx, _) = self.child.as_mut().unwrap();
        let mut line = serde_json::to_string(req).unwrap();
        line.push('\n');
        if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
            let how = self.kill();
            return Outcome::Died(how);
        }
        match rx.recv_timeout(limit) {
            Ok(s) => Outcome::Done(serde_json::from_str(&s).expect("bad worker response")),
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                Outcome::Timeout
            }
            Err(RecvTimeoutError::Disconnected) => {
                // wait for the process so the exit status is known
                let (c, _, _, _) = self.child.as_mut().unwrap();
                let _ = c.wait();
                let how = self.kill();
                Outcome::Died(how)
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop(true);
    }
}
