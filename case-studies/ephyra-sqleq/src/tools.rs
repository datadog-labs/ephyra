// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! sqleq's tools, found from `$SQLEQ_BIN` (a directory with
//! `sqleq-frontend`, `sqleq-fuzz`, `sqleq-solver`; default: PATH) and
//! `$QED_PROVER` (the `qed-prover` binary; default: `qed-prover` on PATH).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn bin(name: &str) -> PathBuf {
    std::env::var_os("SQLEQ_BIN").map_or_else(|| PathBuf::from(name), |dir| PathBuf::from(dir).join(name))
}

pub fn qed() -> PathBuf {
    std::env::var_os("QED_PROVER").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("qed-prover"))
}

/// The frontend on a pair file: Ok(()) when it lowers to `out`, else its
/// first error line.
pub fn lower(pair: &Path, out: &Path) -> Result<(), String> {
    let o = Command::new(bin("sqleq-frontend")).arg(pair).arg(out).output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&o.stderr).into_owned() + &String::from_utf8_lossy(&o.stdout);
        Err(text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string())
    }
}
