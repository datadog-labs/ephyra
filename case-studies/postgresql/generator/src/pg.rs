// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The PostgreSQL server the tests run against, a session per thread, and
//! results as model values.
//!
//! The server is a throwaway cluster of PostgreSQL master built with
//! assertions: binaries in `EPHYRA_PG_BIN` (default
//! `~/Projects/postgres/inst/bin`), data and socket in `EPHYRA_PG_DIR`
//! (default `.pg` in this crate). The first session of a process starts the
//! cluster when it is not running (and creates it when it does not exist:
//! `initdb` with the C locale and UTF-8), and leaves it running for the
//! next process; `pg_ctl -D .pg/data stop` stops it. It runs with fsync,
//! synchronous commit and full-page writes off (nothing here outlives a
//! case), autovacuum off (so no plan depends on when it ran), and dynamic
//! shared memory in files of the data directory (`mmap`): parallel queries'
//! segments in POSIX shared memory failed with "No space left on device" in
//! containers, whose /dev/shm is 64 MB.
//!
//! Each thread has one connection ([`with_session`]) to a database of its
//! own, whose schema [`Session::fresh`] drops and creates again before every
//! case, with the session's settings reset. The databases of processes that
//! are gone are dropped when a process first connects. Results are read in binary ([`Cell`]):
//! reals to the bit, numerics with their scale.

use std::{
    cell::RefCell,
    mem::ManuallyDrop,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use postgres::{
    types::{FromSql, Type as PgType},
    Client, NoTls, SimpleQueryMessage,
};

use crate::model::{Numeric, Row, Value};

/// The directory of the PostgreSQL binaries.
pub fn bin_dir() -> PathBuf {
    match std::env::var_os("EPHYRA_PG_BIN") {
        Some(d) => d.into(),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Projects/postgres/inst/bin"),
    }
}

/// The cluster's directory: `data/` and the socket.
pub fn pg_dir() -> PathBuf {
    match std::env::var_os("EPHYRA_PG_DIR") {
        Some(d) => d.into(),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".pg"),
    }
}

fn connect_to(db: &str) -> Result<Client, postgres::Error> {
    let host = pg_dir();
    let mut config = postgres::Config::new();
    config.host_path(&host).user("postgres").dbname(db).connect_timeout(Duration::from_secs(10));
    config.connect(NoTls)
}

static STARTED: Mutex<bool> = Mutex::new(false);

/// Make sure the cluster runs, starting (and creating) it when needed.
pub fn ensure_server() -> Result<(), String> {
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if *started && connect_to("postgres").is_ok() {
        return Ok(());
    }
    if connect_to("postgres").is_err() {
        start_server()?;
    }
    let mut admin = connect_to("postgres").map_err(|e| format!("connect: {e}"))?;
    drop_stale_databases(&mut admin);
    *started = true;
    Ok(())
}

/// Drop the databases of sessions whose process is gone (`e{pid}_{n}`).
fn drop_stale_databases(admin: &mut Client) {
    let Ok(rows) = admin.query("SELECT datname FROM pg_database WHERE datname ~ '^e[0-9]+_[0-9]+$'", &[]) else {
        return;
    };
    for r in rows {
        let name: String = r.get(0);
        let pid: u32 = name[1..].split('_').next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let alive = Command::new("kill").args(["-0", &pid.to_string()]).output().is_ok_and(|o| o.status.success());
        if !alive {
            let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
        }
    }
}

fn start_server() -> Result<(), String> {
    let dir = pg_dir();
    let data = dir.join("data");
    let bin = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // One process starts it; the others wait for it.
    let lock = dir.join("start.lock");
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => break,
            Err(_) if connect_to("postgres").is_ok() => return Ok(()),
            Err(_) if Instant::now() > deadline => {
                let _ = std::fs::remove_file(&lock);
            },
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    let result = (|| {
        if connect_to("postgres").is_ok() {
            return Ok(());
        }
        if !data.join("PG_VERSION").exists() {
            let out = Command::new(bin.join("initdb"))
                .args(["-D"])
                .arg(&data)
                .args(["-U", "postgres", "--no-sync", "-E", "UTF8", "--locale=C", "-A", "trust"])
                .output()
                .map_err(|e| format!("initdb: {e}"))?;
            if !out.status.success() {
                return Err(format!("initdb: {}", String::from_utf8_lossy(&out.stderr)));
            }
        }
        let opts = format!(
            "-k {} -c listen_addresses= -c fsync=off -c synchronous_commit=off -c full_page_writes=off -c autovacuum=off -c max_connections=200 -c shared_buffers=256MB -c max_locks_per_transaction=256 -c dynamic_shared_memory_type=mmap",
            dir.display()
        );
        let out = Command::new(bin.join("pg_ctl"))
            .arg("-D")
            .arg(&data)
            .arg("-l")
            .arg(dir.join("server.log"))
            .args(["-o", &opts, "-w", "-t", "60", "start"])
            .output()
            .map_err(|e| format!("pg_ctl: {e}"))?;
        if !out.status.success() && connect_to("postgres").is_err() {
            return Err(format!("pg_ctl start: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&lock);
    result
}

/// A value of a result, read in binary: a model value for the types the
/// model has, the raw bytes with the type's name for any other.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Value(Value),
    Other(String, Vec<u8>),
}

struct Raw(Option<Vec<u8>>, PgType);

impl<'a> FromSql<'a> for Raw {
    fn from_sql(ty: &PgType, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(Some(raw.to_vec()), ty.clone()))
    }

    fn from_sql_null(ty: &PgType) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(None, ty.clone()))
    }

    fn accepts(_: &PgType) -> bool {
        true
    }
}

fn be<const N: usize>(b: &[u8]) -> Result<[u8; N], String> {
    b.try_into().map_err(|_| format!("{} bytes where {N} were expected", b.len()))
}

/// The binary form of a numeric (`numeric_send`): digit count, weight (of
/// the first base-10000 digit), sign, display scale, digits.
pub fn decode_numeric(b: &[u8]) -> Result<Numeric, String> {
    if b.len() < 8 {
        return Err("short numeric".into());
    }
    let u16_at = |at: usize| u16::from_be_bytes([b[at], b[at + 1]]);
    let ndigits = usize::from(u16_at(0));
    let weight = i64::from(u16_at(2) as i16);
    let sign = u16_at(4);
    let dscale = u16_at(6);
    match sign {
        0xC000 => return Ok(Numeric::NaN),
        0xD000 => return Ok(Numeric::Inf),
        0xF000 => return Ok(Numeric::NegInf),
        0x0000 | 0x4000 => {},
        _ => return Err(format!("numeric sign {sign:#x}")),
    }
    if b.len() != 8 + 2 * ndigits {
        return Err("numeric length".into());
    }
    let groups: Vec<u16> = (0..ndigits).map(|k| u16_at(8 + 2 * k)).collect();
    // Digits of the integer part (groups 0..=weight) and of the fraction.
    let mut int = String::new();
    for k in 0..=weight.max(-1) {
        let g = groups.get(k as usize).copied().unwrap_or(0);
        int.push_str(&format!("{g:04}"));
    }
    let mut frac = String::new();
    let mut k = weight + 1;
    while (frac.len() as u64) < u64::from(dscale) {
        let g = if k >= 0 { groups.get(k as usize).copied().unwrap_or(0) } else { 0 };
        frac.push_str(&format!("{g:04}"));
        k += 1;
    }
    frac.truncate(usize::from(dscale));
    let digits: Vec<u8> = int.bytes().chain(frac.bytes()).collect();
    Ok(Numeric::finite(sign == 0x4000, digits, dscale))
}

/// A result value as a cell.
fn decode(raw: Raw) -> Result<Cell, String> {
    let Raw(bytes, ty) = raw;
    let Some(b) = bytes else {
        return Ok(Cell::Value(Value::Null));
    };
    let v = match ty {
        PgType::INT2 => Value::Int(i16::from_be_bytes(be(&b)?).into()),
        PgType::INT4 => Value::Int(i32::from_be_bytes(be(&b)?).into()),
        PgType::INT8 => Value::Int(i64::from_be_bytes(be(&b)?)),
        PgType::FLOAT4 => Value::Float(f32::from_be_bytes(be(&b)?).into()),
        PgType::FLOAT8 => Value::Float(f64::from_be_bytes(be(&b)?)),
        PgType::NUMERIC => Value::Numeric(decode_numeric(&b)?),
        PgType::TEXT | PgType::VARCHAR | PgType::BPCHAR | PgType::NAME => Value::Text(String::from_utf8(b).map_err(|e| e.to_string())?),
        PgType::BOOL => Value::Bool(b == [1]),
        PgType::BYTEA => Value::Bytea(b),
        PgType::DATE => Value::Date(i32::from_be_bytes(be(&b)?)),
        PgType::TIMESTAMP => Value::Timestamp(i64::from_be_bytes(be(&b)?)),
        other => return Ok(Cell::Other(other.name().to_string(), b)),
    };
    Ok(Cell::Value(v))
}

/// An error as one line: the server's message (with its SQLSTATE), or the
/// client's.
pub fn message(e: &postgres::Error) -> String {
    match e.as_db_error() {
        Some(d) => format!("{} ({})", d.message(), d.code().code()),
        None => e.to_string(),
    }
}

/// Whether an error means the server went away: a crash (an assertion
/// failure, in this build), never a statement's own error.
pub fn lost(e: &str) -> bool {
    e.contains("connection closed") || e.contains("terminating connection") || e.contains("server closed") || e.contains("57P01") || e.contains("57P02") || e.contains("57P03")
}

static SESSIONS: AtomicU64 = AtomicU64::new(0);

/// A thread's connection, to a database of its own (`e{pid}_{n}`): a
/// database-wide VACUUM or ANALYZE, and REINDEX CONCURRENTLY's wait for
/// older snapshots, see only the sessions of one database, so threads never
/// wait for each other.
pub struct Session {
    pub client: Client,
    pub database: String,
    pub schema: String,
    resets: u64,
}

/// The cluster runs with autovacuum off (its ANALYZE would change plans
/// under the oracles), so nothing vacuums a session's catalogs: each input's
/// DROP SCHEMA leaves a few hundred dead rows in pg_attribute, pg_class,
/// pg_depend, .., and after a few hours a database's catalogs pass 1 GB and
/// throughput halves. Every `CATALOG_VACUUM_EVERY` resets they are vacuumed
/// by name (a shared catalog's VACUUM waits on other databases' sessions),
/// from a connection of its own: a VACUUM in the session's backend would
/// move its pruning horizon for the next input (as in the TOAST
/// missing-chunk bug).
const CATALOG_VACUUM_EVERY: u64 = 200;
const CATALOGS: &str = "pg_catalog.pg_attribute, pg_catalog.pg_class, pg_catalog.pg_depend, pg_catalog.pg_type, \
    pg_catalog.pg_index, pg_catalog.pg_constraint, pg_catalog.pg_attrdef, pg_catalog.pg_statistic, \
    pg_catalog.pg_statistic_ext, pg_catalog.pg_statistic_ext_data, pg_catalog.pg_namespace, pg_catalog.pg_rewrite, \
    pg_catalog.pg_inherits, pg_catalog.pg_sequence, pg_catalog.pg_init_privs, pg_catalog.pg_partitioned_table";

impl Session {
    fn open() -> Result<Session, String> {
        ensure_server()?;
        let database = format!("e{}_{}", std::process::id(), SESSIONS.fetch_add(1, Ordering::Relaxed));
        let mut admin = connect_to("postgres").map_err(|e| format!("connect: {}", message(&e)))?;
        // One statement each: several would run as one transaction, where
        // neither may.
        admin.batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)")).map_err(|e| format!("drop database: {}", message(&e)))?;
        admin.batch_execute(&format!("CREATE DATABASE {database} TEMPLATE template0")).map_err(|e| format!("create database: {}", message(&e)))?;
        let mut client = connect_to(&database).map_err(|e| format!("connect: {}", message(&e)))?;
        client.batch_execute("CREATE EXTENSION amcheck; CREATE EXTENSION btree_gin; CREATE EXTENSION btree_gist").map_err(|e| format!("extensions: {}", message(&e)))?;
        Ok(Session { client, database, schema: "s".into(), resets: 0 })
    }

    /// An empty schema, the session's settings reset (a statement timeout
    /// of 10 s), no transaction open.
    pub fn fresh(&mut self) -> Result<(), String> {
        let s = &self.schema;
        self.client
            .batch_execute(&format!(
                "ROLLBACK; RESET ALL; SET client_min_messages = warning; SET statement_timeout = '10s'; DROP SCHEMA IF EXISTS {s} CASCADE; CREATE SCHEMA {s}; SET search_path = {s}, public"
            ))
            .map_err(|e| message(&e))?;
        self.resets += 1;
        if self.resets % CATALOG_VACUUM_EVERY == 0 {
            // Best effort: a failed VACUUM only means more bloat.
            if let Ok(mut c) = connect_to(&self.database) {
                let _ = c.batch_execute(&format!("VACUUM {CATALOGS}"));
            }
        }
        Ok(())
    }

    /// Run a statement (or several, `;`-separated) and return the row count
    /// of the last one's command tag.
    pub fn execute(&mut self, sql: &str) -> Result<u64, String> {
        let messages = self.client.simple_query(sql).map_err(|e| message(&e))?;
        Ok(messages
            .iter()
            .filter_map(|m| match m {
                SimpleQueryMessage::CommandComplete(n) => Some(*n),
                _ => None,
            })
            .last()
            .unwrap_or(0))
    }

    /// Run a query and return its rows as cells.
    pub fn query(&mut self, sql: &str) -> Result<Vec<Vec<Cell>>, String> {
        let rows = self.client.query(sql, &[]).map_err(|e| message(&e))?;
        rows.iter().map(|r| (0..r.len()).map(|k| decode(r.try_get::<_, Raw>(k).map_err(|e| e.to_string())?)).collect()).collect()
    }

    /// Run a query whose values are all of the model's types.
    pub fn values(&mut self, sql: &str) -> Result<Vec<Row>, String> {
        self.query(sql)?
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|c| match c {
                        Cell::Value(v) => Ok(v),
                        Cell::Other(ty, _) => Err(format!("a value of type {ty}")),
                    })
                    .collect()
            })
            .collect()
    }

    /// `COPY t (..) FROM STDIN` with `data` in text format.
    pub fn copy_in(&mut self, sql: &str, data: &str) -> Result<u64, String> {
        use std::io::Write;
        let mut w = self.client.copy_in(sql).map_err(|e| message(&e))?;
        w.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
        w.finish().map_err(|e| message(&e))
    }
}

thread_local! {
    /// Never dropped: the client's runtime is gone by the time thread-local
    /// destructors run, and dropping it then aborts. The connection closes
    /// with the process; [`ensure_server`] drops the databases of processes
    /// that are gone.
    static SESSION: RefCell<Option<ManuallyDrop<Session>>> = const { RefCell::new(None) };
}

/// Run `f` on this thread's session, connecting first when there is none or
/// the last one was lost.
pub fn with_session<T>(f: impl FnOnce(&mut Session) -> Result<T, String>) -> Result<T, String> {
    SESSION.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().is_none_or(|s| s.client.is_closed()) {
            *slot = Some(ManuallyDrop::new(Session::open()?));
        }
        f(slot.as_mut().unwrap())
    })
}

/// The process id of this thread's backend, when it has a session.
pub fn backend_pid() -> Option<i32> {
    SESSION.with(|cell| {
        let mut slot = cell.borrow_mut();
        let s = slot.as_mut()?;
        s.client.query_one("SELECT pg_backend_pid()", &[]).ok().map(|r| r.get(0))
    })
}

/// Close this thread's connection and drop its database (at the end of a
/// run).
pub fn close_session() {
    SESSION.with(|cell| {
        if let Some(s) = cell.borrow_mut().take() {
            let s = ManuallyDrop::into_inner(s);
            let database = s.database.clone();
            drop(s);
            if let Ok(mut admin) = connect_to("postgres") {
                let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Type, DATE_MAX, DATE_MIN, TIMESTAMP_MAX, TIMESTAMP_MIN};

    #[test]
    fn values_read_back_exactly() {
        with_session(|s| {
            s.fresh()?;
            let n = |x: &str| Value::Numeric(Numeric::parse(x).unwrap());
            let cases: Vec<(Type, Value)> = vec![
                (Type::Int2, Value::Int(i64::from(i16::MIN))),
                (Type::Int8, Value::Int(i64::MIN)),
                (Type::Float8, Value::Float(-0.0)),
                (Type::Float8, Value::Float(5e-324)),
                (Type::Float8, Value::Float(f64::MAX)),
                (Type::Float8, Value::Float(f64::NEG_INFINITY)),
                (Type::Float4, Value::Float(f64::from(0.1f32))),
                (Type::Float4, Value::Float(f64::from(f32::from_bits(1)))),
                (Type::Numeric, n("0.00")),
                (Type::Numeric, n("-12345678901234567890.123456789")),
                (Type::Numeric, n("0.00001000")),
                (Type::Numeric, n("100000000")),
                (Type::Numeric, n("NaN")),
                (Type::Numeric, n("-Infinity")),
                (Type::Text, Value::Text("a'b\\c\u{e9}\u{1f600}".into())),
                (Type::Bool, Value::Bool(true)),
                (Type::Bytea, Value::Bytea(vec![0, 39, 92, 255])),
                (Type::Date, Value::Date(DATE_MIN)),
                (Type::Date, Value::Date(DATE_MAX)),
                (Type::Date, Value::Date(i32::MIN)),
                (Type::Date, Value::Date(-730_120)),
                (Type::Timestamp, Value::Timestamp(TIMESTAMP_MIN)),
                (Type::Timestamp, Value::Timestamp(TIMESTAMP_MAX)),
                (Type::Timestamp, Value::Timestamp(-1)),
                (Type::Timestamp, Value::Timestamp(i64::MAX)),
            ];
            for (ty, v) in cases {
                assert!(ty.admits(&v), "{ty:?} {v:?}");
                let sql = format!("SELECT {}::{}", crate::statements::literal(&v), ty.name());
                let got = s.values(&sql)?;
                assert!(crate::model::identical(&got[0][0], &v), "{sql}: {:?}", got[0][0]);
            }
            Ok(())
        })
        .unwrap();
    }
}
