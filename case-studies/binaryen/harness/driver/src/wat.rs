// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Standard text format from a second implementation: wasmprinter
//! (wasm-tools 0.261, the release wasm-smith and wasmtime 50 use), for
//! oracle text-read (fuzz.rs).

/// `wasm` as text. `fold`: folded (s-expression) instructions instead of
/// flat ones; `name_unnamed`: synthesized `$#func0`-style identifiers for
/// unnamed items, so references are by name instead of by index.
pub fn print(wasm: &[u8], fold: bool, name_unnamed: bool) -> Option<String> {
    let mut c = wasmprinter::Config::new();
    c.fold_instructions(fold);
    c.name_unnamed(name_unnamed);
    let mut s = String::new();
    c.print(wasm, &mut wasmprinter::PrintFmtWrite(&mut s)).ok()?;
    Some(s)
}
