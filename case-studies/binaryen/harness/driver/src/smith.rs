// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Second module generator: wasm-smith (wasm-tools), configured for the
//! features of the case. Its modules are shaped unlike -ttf output: arbitrary
//! stack code (multivalue blocks with params, unreachable stack-polymorphic
//! tails), non-canonical LEBs, several tables/memories/tags, GC types in rec
//! groups, element/data segments of every encoding, table initializers.
//! No imports; everything is exported, so bynexec calls every function and
//! prints every global; NaNs are canonicalized by wasm-smith and loops and
//! calls are bounded by its fuel global (a deterministic trap).

use crate::Rng;
use arbitrary::Unstructured;
use wasm_smith::{Config, Module};

fn on(features: &[String], f: &str) -> bool {
    !features.iter().any(|x| x == &format!("--disable-{f}"))
}

/// A wasm-smith module for `features` (wasm-opt flags), or None when
/// wasm-smith ran out of input or failed.
pub fn module(rng: &mut Rng, features: &[String]) -> Option<Vec<u8>> {
    let len = 512usize << rng.below(6);
    let len = len + rng.below(len);
    let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
    let mut u = Unstructured::new(&bytes);

    let mut c = Config::default();
    c.max_imports = 0;
    c.available_imports = None;
    c.export_everything = true;
    c.canonicalize_nans = true;
    c.allow_start_export = true;
    c.min_funcs = 1;
    c.max_funcs = 1 + rng.below(30);
    c.max_instructions = 20 + rng.below(800);
    c.max_types = 1 + rng.below(40);
    c.max_type_size = 100;
    c.max_globals = rng.below(30);
    c.max_tags = rng.below(5);
    c.max_data_segments = rng.below(10);
    c.max_element_segments = rng.below(10);
    c.max_elements = rng.below(50);
    c.max_tables = 1 + rng.below(4);
    c.max_memories = if on(features, "multimemory") { 1 + rng.below(4) } else { 1 };
    c.max_table_elements = 1 + rng.below(1000) as u64;
    c.table_max_size_required = rng.chance(0.3);
    c.memory_max_size_required = rng.chance(0.3);
    // Binaryen's interpreter allocates memories eagerly; keep them small.
    c.max_memory32_bytes = 1 << (12 + rng.below(9));
    c.max_memory64_bytes = 1 << (12 + rng.below(9));
    c.max_nesting_depth = 1 + rng.below(10);
    c.min_uleb_size = if rng.chance(0.2) { 1 + rng.below(5) as u8 } else { 1 };
    c.disallow_traps = rng.chance(0.3);
    c.generate_custom_sections = rng.chance(0.1);

    c.simd_enabled = on(features, "simd");
    c.relaxed_simd_enabled = false;
    c.reference_types_enabled = on(features, "reference-types");
    c.gc_enabled = on(features, "gc") && c.reference_types_enabled;
    c.multi_value_enabled = on(features, "multivalue");
    c.exceptions_enabled = on(features, "exception-handling");
    c.tail_call_enabled = on(features, "tail-call");
    c.threads_enabled = on(features, "threads");
    c.memory64_enabled = on(features, "memory64");
    c.custom_page_sizes_enabled = on(features, "custom-page-sizes");
    // Binaryen represents the two results of i64.mul_wide_* etc. as a tuple,
    // which its validator allows only with multivalue.
    c.wide_arithmetic_enabled = on(features, "wide-arithmetic") && c.multi_value_enabled;
    c.bulk_memory_enabled = true;
    c.saturating_float_to_int_enabled = true;
    c.sign_extension_ops_enabled = true;
    c.extended_const_enabled = true;
    c.compact_imports_enabled = false;
    c.shared_everything_threads_enabled = false;
    c.custom_descriptors_enabled = false;
    if !c.reference_types_enabled {
        c.max_tables = 1;
    }

    let mut m = Module::new(c, &mut u).ok()?;
    m.ensure_termination(100 + rng.below(5000) as u32).ok()?;
    Some(m.to_bytes())
}

/// The features of `features` (wasm-opt flags) for wasmparser, to drop
/// modules wasm-smith made outside its configuration.
pub fn wasmparser_features(features: &[String]) -> wasmparser::WasmFeatures {
    use wasmparser::WasmFeatures as F;
    let mut w = F::all();
    w.remove(F::RELAXED_SIMD | F::SHARED_EVERYTHING_THREADS | F::CUSTOM_DESCRIPTORS | F::LEGACY_EXCEPTIONS | F::COMPACT_IMPORTS);
    for (name, flag) in [
        ("simd", F::SIMD),
        ("reference-types", F::REFERENCE_TYPES),
        ("gc", F::GC),
        ("multivalue", F::MULTI_VALUE),
        ("exception-handling", F::EXCEPTIONS),
        ("tail-call", F::TAIL_CALL),
        ("threads", F::THREADS),
        ("memory64", F::MEMORY64),
        ("multimemory", F::MULTI_MEMORY),
        ("custom-page-sizes", F::CUSTOM_PAGE_SIZES),
        ("wide-arithmetic", F::WIDE_ARITHMETIC),
    ] {
        if !on(features, name) {
            w.remove(flag);
        }
    }
    w
}

/// Whether the module has f64.promote_f32 / f32.demote_f64 (or their SIMD
/// forms). wasm-smith's canonicalize_nans does not cover them, and their NaN
/// results are nondeterministic (spec: promote/demote of a NaN gives any
/// arithmetic NaN; a load can make a NaN with a payload). Such modules get
/// only the crash oracles, like -ttf modules without --denan.
pub fn has_nondeterministic_nans(bytes: &[u8]) -> bool {
    use wasmparser::{Operator, Parser, Payload};
    for payload in Parser::new(0).parse_all(bytes) {
        let Ok(Payload::CodeSectionEntry(body)) = payload else { continue };
        let Ok(mut ops) = body.get_operators_reader() else { return true };
        while !ops.eof() {
            match ops.read() {
                Ok(Operator::F64PromoteF32 | Operator::F32DemoteF64 | Operator::F64x2PromoteLowF32x4 | Operator::F32x4DemoteF64x2Zero) => return true,
                Ok(_) => {}
                Err(_) => return true,
            }
        }
    }
    false
}

/// Whether the module has a declared element segment. Binaryen's reader
/// drops declared segments, so later segment indices shift and elem.drop of a
/// declared segment points past the end (an open bug, "invalid element
/// segment index"); most such modules are regenerated so the bug does not
/// drown everything else.
pub fn has_declared_elem(bytes: &[u8]) -> bool {
    use wasmparser::{ElementKind, Parser, Payload};
    for payload in Parser::new(0).parse_all(bytes) {
        let Ok(Payload::ElementSection(r)) = payload else { continue };
        for e in r {
            if matches!(e, Ok(e) if matches!(e.kind, ElementKind::Declared)) {
                return true;
            }
        }
    }
    false
}

/// Whether the module defines a table with an initializer expression.
pub fn has_table_init(bytes: &[u8]) -> bool {
    use wasmparser::{Parser, Payload, TableInit};
    for payload in Parser::new(0).parse_all(bytes) {
        let Ok(Payload::TableSection(r)) = payload else { continue };
        for t in r {
            if matches!(t, Ok(t) if matches!(t.init, TableInit::Expr(_))) {
                return true;
            }
        }
    }
    false
}
