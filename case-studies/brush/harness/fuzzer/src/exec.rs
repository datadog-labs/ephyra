// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Running one shell on one script in a scratch directory, with a timeout.

use std::{
    io::Read,
    os::unix::process::{CommandExt, ExitStatusExt},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const OUT_CAP: usize = 1 << 20;

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Snapshot of the scratch directory after the run: (relative path, contents); dirs end in '/'.
    pub files: Vec<(String, Vec<u8>)>,
}

pub struct Shell {
    pub bin: String,
    pub args: Vec<String>,
}

impl Shell {
    pub fn brush(bin: &str) -> Shell {
        // Flags from brush-test-harness/src/config.rs (how brush's own compat tests run it).
        let args = ["--norc", "--noprofile", "--no-config", "--disable-bracketed-paste", "--disable-color"];
        Shell { bin: bin.into(), args: args.iter().map(|s| s.to_string()).collect() }
    }
    pub fn bash(bin: &str) -> Shell {
        Shell { bin: bin.into(), args: vec!["--norc".into(), "--noprofile".into()] }
    }
}

fn reader(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out.len() < OUT_CAP {
                        out.extend_from_slice(&buf[..n]);
                    }
                }
            }
        }
        out
    })
}

pub struct Running {
    child: std::process::Child,
    out: Option<std::thread::JoinHandle<Vec<u8>>>,
    err: Option<std::thread::JoinHandle<Vec<u8>>>,
    deadline: Instant,
}

/// Start `shell script` with cwd `dir` (must exist and be empty) and a cleared environment.
pub fn start(shell: &Shell, script: &Path, dir: &Path, limit: Duration) -> std::io::Result<Running> {
    let mut cmd = Command::new(&shell.bin);
    cmd.args(&shell.args)
        // relative path: $0 (reachable via ${@: -1} with no positional args) is then the same
        // in the search, in repro and for both shells
        .arg(Path::new("..").join(script.file_name().unwrap()))
        .current_dir(dir)
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent-home")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // Coverage builds: keep LLVM_PROFILE_FILE (made absolute, since cwd is the scratch dir).
    if let Some(p) = std::env::var_os("LLVM_PROFILE_FILE") {
        cmd.env("LLVM_PROFILE_FILE", profile_pattern(&p.to_string_lossy())?);
    }
    let mut child = cmd.spawn()?;
    let out = Some(reader(child.stdout.take().unwrap()));
    let err = Some(reader(child.stderr.take().unwrap()));
    Ok(Running { child, out, err, deadline: Instant::now() + limit })
}

impl Running {
    pub fn finish(mut self, dir: &Path) -> Outcome {
        let pid = self.child.id() as i32;
        let mut timed_out = false;
        let status = loop {
            if let Ok(Some(st)) = self.child.try_wait() {
                break st;
            }
            if Instant::now() > self.deadline {
                timed_out = true;
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                break self.child.wait().expect("wait");
            }
            std::thread::sleep(Duration::from_micros(300));
        };
        // Leftover background processes in the group would keep the pipes open.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
        let stdout = self.out.take().unwrap().join().unwrap_or_default();
        let stderr = self.err.take().unwrap().join().unwrap_or_default();
        let files = snapshot(dir);
        Outcome {
            code: status.code(),
            signal: if timed_out { None } else { status.signal() },
            timed_out,
            stdout,
            stderr,
            files,
        }
    }
}

pub fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = format!("{prefix}{}", e.file_name().to_string_lossy());
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            out.push((format!("{name}/"), vec![]));
            if out.len() < 200 {
                walk(&e.path(), &format!("{name}/"), out);
            }
        } else {
            let mut data = std::fs::read(e.path()).unwrap_or_default();
            data.truncate(65536);
            out.push((name, data));
        }
    }
}

/// Write the script both shells run, read-only. `$0` is the script (reachable via `${@: -1}`
/// with no positional args), so `cmd > ${@: -1}` would otherwise truncate it: bash reads its
/// script as it goes and stops, brush has parsed it all already, and both shells share the file.
/// Read-only, the redirection fails the same way in both shells (EACCES; the search is not root).
pub fn write_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    std::fs::write(path, script).expect("write script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).expect("chmod script");
}

pub fn reset_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).expect("mkdir scratch");
}

/// One profile file per process (`%p`) means ~10 GB per 30 s here. Replace `%p` by this
/// worker thread's id and make sure `%m` is present, so the LLVM runtime merges every run
/// of one worker into one file (online merging, with file locking for forked subshells).
fn profile_pattern(p: &str) -> std::io::Result<std::path::PathBuf> {
    let tid = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let mut s = p.replace("%p", &format!("{}-{tid}", std::process::id()));
    if !s.contains("%m") && !regex_like_nm(&s) {
        s.push_str("%m");
    }
    let path = std::path::PathBuf::from(s);
    Ok(if path.is_absolute() { path } else { std::env::current_dir()?.join(path) })
}

/// true if the pattern already has a `%<digits>m` merge-pool specifier
fn regex_like_nm(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).any(|i| {
        b[i] == b'%' && {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() { j += 1; }
            j > i + 1 && j < b.len() && b[j] == b'm'
        }
    })
}
