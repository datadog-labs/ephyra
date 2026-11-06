// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! CREATE VIRTUAL TABLE IF NOT EXISTS fails with a syntax error and
//! creates nothing.
use turso_apidiff_repro2::{exec, open};
fn main() {
    let c = open("/tmp/repro2-vtab-ifne.db");
    exec(&c, "CREATE VIRTUAL TABLE IF NOT EXISTS v USING mem");
    exec(&c, "SELECT name, sql FROM sqlite_schema");
    exec(&c, "CREATE VIRTUAL TABLE w USING mem");
    exec(&c, "SELECT name, sql FROM sqlite_schema");
}
