// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! bynexec <m.wasm>: run a binaryen -ttf module in wasmtime the way binaryen's
//! fuzzer runs it (src/tools/execution-results.h, scripts/fuzz_shell.js):
//! provide the `fuzzing-support` imports, call every export in order with
//! default arguments, and print one line per observable event.
//!
//! The output is normalised so that it is equal before and after any
//! semantics-preserving optimization:
//! - NaN sign/payload is nondeterministic in wasm (spec, NaN propagation;
//!   binaryen src/passes/Precompute.cpp:24-27): f32/f64 NaNs print as `nan`,
//!   v128 lanes that hold a NaN pattern print as `nan32`/`nan64`.
//! - Trap kinds may change (fuzz_opt.py maps all to one): every trap is `trap`.
//! - Reference identities (function indices, types) are not observable: refs
//!   print as null / function / object / i31(N) / externref(N).
//!
//! Exit codes: 0 ran (output on stdout), 3 hit a host limit (stack, fuel, GC
//! heap, allocation size; the comparison must be ignored, as binaryen's
//! ExecutionResults::ignore does), 4 could not compile or link the module.

use std::process::exit;
use wasmtime::*;

const JS_ERROR_PAYLOAD: u32 = 0xbad;
const FUEL: u64 = 200_000_000;

struct Host {
    out: Vec<String>,
    /// All exports in module order; None while instantiating (start function).
    exports: Option<Vec<Extern>>,
    table: Option<Table>,
    wasmtag: Option<Tag>,
    jstag: Option<Tag>,
    temp_ret0: i32,
    limits: StoreLimits,
}

fn is_nan32(b: u32) -> bool {
    (b & 0x7f80_0000) == 0x7f80_0000 && (b & 0x007f_ffff) != 0
}
fn is_nan64(b: u64) -> bool {
    (b & 0x7ff0_0000_0000_0000) == 0x7ff0_0000_0000_0000 && (b & 0x000f_ffff_ffff_ffff) != 0
}

fn fmt_f32(b: u32) -> String {
    if is_nan32(b) { "nan".into() } else { format!("{:?}", f32::from_bits(b)) }
}
fn fmt_f64(b: u64) -> String {
    if is_nan64(b) { "nan".into() } else { format!("{:?}", f64::from_bits(b)) }
}

fn fmt_v128(v: u128) -> String {
    // The lane interpretation is unknown, so mask any 64-bit half that is an
    // f64 NaN, and otherwise any 32-bit lane that is an f32 NaN.
    let mut parts = Vec::new();
    for half in 0..2 {
        let h = (v >> (64 * half)) as u64;
        if is_nan64(h) {
            parts.push("nan64".to_string());
            continue;
        }
        for lane in 0..2 {
            let l = (h >> (32 * lane)) as u32;
            parts.push(if is_nan32(l) { "nan32".to_string() } else { format!("{l:08x}") });
        }
    }
    format!("v128({})", parts.join(" "))
}

fn fmt_anyref(store: impl AsContext, r: &Rooted<AnyRef>) -> String {
    match r.as_i31(&store) {
        Ok(Some(i)) => format!("i31({})", i.get_i32()),
        _ => "object".into(),
    }
}

fn fmt_val(mut store: impl AsContextMut<Data = Host>, v: &Val) -> String {
    match v {
        Val::I32(x) => x.to_string(),
        Val::I64(x) => x.to_string(),
        Val::F32(b) => fmt_f32(*b),
        Val::F64(b) => fmt_f64(*b),
        Val::V128(x) => fmt_v128(x.as_u128()),
        Val::FuncRef(None)
        | Val::ExternRef(None)
        | Val::AnyRef(None)
        | Val::ExnRef(None)
        | Val::ContRef(None) => "null".into(),
        Val::FuncRef(Some(_)) => "function".into(),
        Val::ExternRef(Some(r)) => {
            let payload = match r.data(&store) {
                Ok(Some(d)) => d.downcast_ref::<u32>().copied(),
                _ => None,
            };
            match payload {
                Some(JS_ERROR_PAYLOAD) => "jserror".into(),
                Some(p) => format!("externref({p})"),
                None => match AnyRef::convert_extern(&mut store, *r) {
                    Ok(a) => fmt_anyref(&store, &a),
                    Err(_) => "object".into(),
                },
            }
        }
        Val::AnyRef(Some(r)) => fmt_anyref(&store, r),
        Val::ExnRef(Some(_)) | Val::ContRef(Some(_)) => "object".into(),
    }
}

fn traps_on_boundary(t: &ValType) -> bool {
    match t {
        ValType::V128 => true,
        ValType::Ref(r) => matches!(r.heap_type().top(), HeapTopType::Exn | HeapTopType::Cont),
        _ => false,
    }
}

fn is_cont(t: &ValType) -> bool {
    matches!(t, ValType::Ref(r) if r.heap_type().top() == HeapTopType::Cont)
}

/// wasmtime 50 cannot pass continuation refs through its embedding API (it
/// panics: "embedding API for `(ref cont)`", wasmtime#10248), so functions
/// with cont params/results are never called from or into the host.
fn has_cont(ft: &FuncType) -> bool {
    ft.params().any(|t| is_cont(&t)) || ft.results().any(|t| is_cont(&t))
}

fn throw_tag(caller: &mut Caller<'_, Host>, js: bool, which: i32) -> Result<()> {
    let tag = if js { caller.data().jstag } else { caller.data().wasmtag };
    let tag = tag.ok_or_else(|| format_err!("no tag"))?;
    let field = if js {
        Val::ExternRef(Some(ExternRef::new(&mut *caller, JS_ERROR_PAYLOAD)?))
    } else {
        Val::I32(which)
    };
    let ty = ExnType::from_tag_type(&tag.ty(&*caller))?;
    let pre = ExnRefPre::new(&mut *caller, ty);
    let exn = ExnRef::new(&mut *caller, &pre, &tag, &[field])?;
    caller.as_context_mut().throw(exn)
}

/// Call a function the way JS would (execution-results.h callFunctionAsJS):
/// default arguments, and a JS exception for params/results JS cannot pass.
fn call_as_js(caller: &mut Caller<'_, Host>, f: Func) -> Result<()> {
    let ty = f.ty(&*caller);
    let mut args = Vec::new();
    for p in ty.params() {
        if matches!(p, ValType::I64) || traps_on_boundary(&p) {
            return throw_tag(caller, true, 0);
        }
        match Val::default_for_ty(&p) {
            Some(v) => args.push(v),
            None => return throw_tag(caller, true, 0),
        }
    }
    let mut results = Vec::new();
    for r in ty.results() {
        if traps_on_boundary(&r) {
            return throw_tag(caller, true, 0);
        }
        results.push(Val::default_for_ty(&r).unwrap_or(Val::I32(0)));
    }
    f.call(&mut *caller, &args, &mut results)
}

fn export_func(caller: &mut Caller<'_, Host>, index: i32) -> Option<Func> {
    let exports = caller.data().exports.as_ref()?;
    match exports.get(index as u32 as usize)? {
        Extern::Func(f) => Some(*f),
        _ => None,
    }
}

/// Catch an exception (not a trap) as call-export-catch does.
fn catching(caller: &mut Caller<'_, Host>, r: Result<()>) -> Result<i32> {
    match r {
        Ok(()) => Ok(0),
        Err(e) if e.is::<ThrownException>() => {
            caller.as_context_mut().take_pending_exception();
            Ok(1)
        }
        Err(e) => Err(e),
    }
}

fn host_func(store: &mut Store<Host>, module: &str, name: &str, ft: FuncType) -> Func {
    let module = module.to_string();
    let name = name.to_string();
    if has_cont(&ft) {
        // SAFETY: the closure reads no params and writes no results.
        return unsafe {
            Func::new_unchecked(&mut *store, ft, move |mut caller, _raw| {
                if name.starts_with("log") {
                    caller.data_mut().out.push("[LoggingExternalInterface logging contref]".into());
                    return Ok(());
                }
                Err(format_err!("cont at the host boundary"))
            })
        };
    }
    Func::new(&mut *store, ft.clone(), move |mut caller, params, results| {
        let set0 = |results: &mut [Val], v: Val| {
            if let Some(r) = results.get_mut(0) {
                *r = v;
            }
        };
        if module == "fuzzing-support" {
            if name == "log-branch" {
                let line = format!(
                    "[LoggingExternalInterface log-branch {} {} {}]",
                    fmt_val(&mut caller, &params[0]),
                    fmt_val(&mut caller, &params[1]),
                    fmt_val(&mut caller, &params[2])
                );
                caller.data_mut().out.push(line);
                set0(results, params[0].clone());
                return Ok(());
            }
            if name.starts_with("log") {
                let mut line = String::from("[LoggingExternalInterface logging");
                let split = std::env::var_os("BYNEXEC_SPLIT_I64").is_some();
                for p in params {
                    line.push(' ');
                    match p {
                        // As execution-results.h logs an i64: two i32s, low then high.
                        Val::I64(x) if split => line.push_str(&format!("{} {}", *x as i32, (*x >> 32) as i32)),
                        _ => line.push_str(&fmt_val(&mut caller, p)),
                    }
                }
                line.push(']');
                caller.data_mut().out.push(line);
                return Ok(());
            }
            match name.as_str() {
                "throw" => {
                    let which = params[0].unwrap_i32();
                    return throw_tag(&mut caller, which == 0, which);
                }
                "table-get" | "table-set" => {
                    let table = match (&caller.data().exports, caller.data().table) {
                        (Some(_), Some(t)) => t,
                        _ => return throw_tag(&mut caller, true, 0),
                    };
                    let index = params[0].unwrap_i32() as u32 as u64;
                    let cont_table = table.ty(&caller).element().heap_type().top() == HeapTopType::Cont;
                    if cont_table || index >= table.size(&caller) {
                        return throw_tag(&mut caller, true, 0);
                    }
                    if name == "table-get" {
                        let v = table.get(&mut caller, index).ok_or_else(|| format_err!("table.get"))?;
                        set0(results, v.into());
                    } else {
                        let r = params[1].ref_().ok_or_else(|| format_err!("table-set arg"))?;
                        table.set(&mut caller, index, r)?;
                    }
                    return Ok(());
                }
                "call-export" | "call-export-catch" => {
                    let r = match export_func(&mut caller, params[0].unwrap_i32()) {
                        Some(f) => call_as_js(&mut caller, f),
                        None => throw_tag(&mut caller, true, 0),
                    };
                    if name == "call-export" {
                        return r;
                    }
                    let c = catching(&mut caller, r)?;
                    set0(results, Val::I32(c));
                    return Ok(());
                }
                "call-ref" | "call-ref-catch" => {
                    let r = match &params[0] {
                        Val::FuncRef(Some(f)) => call_as_js(&mut caller, *f),
                        _ => throw_tag(&mut caller, true, 0),
                    };
                    if name == "call-ref" {
                        return r;
                    }
                    let c = catching(&mut caller, r)?;
                    set0(results, Val::I32(c));
                    return Ok(());
                }
                "sleep" => {
                    set0(results, params[1].clone());
                    return Ok(());
                }
                _ => {}
            }
        } else if module == "env" {
            if name == "setTempRet0" {
                caller.data_mut().temp_ret0 = params[0].unwrap_i32();
                return Ok(());
            }
            if name == "getTempRet0" {
                let v = caller.data().temp_ret0;
                set0(results, Val::I32(v));
                return Ok(());
            }
        }
        // Unknown import: binaryen ignores it; return defaults.
        for (i, t) in ft.results().enumerate() {
            results[i] = Val::default_for_ty(&t).unwrap_or(Val::I32(0));
        }
        Ok(())
    })
}

enum Outcome {
    Trap,
    Exception,
    Limit(String),
}

fn classify(store: &mut Store<Host>, e: &Error) -> Outcome {
    if e.is::<ThrownException>() {
        store.take_pending_exception();
        return Outcome::Exception;
    }
    if let Some(t) = e.downcast_ref::<Trap>() {
        return match t {
            Trap::StackOverflow | Trap::OutOfFuel | Trap::AllocationTooLarge | Trap::Interrupt => {
                Outcome::Limit(format!("{t:?}"))
            }
            _ => Outcome::Trap,
        };
    }
    let msg = format!("{e:?}");
    if msg.contains("out of memory") || msg.contains("GC heap") || msg.contains("stack") || msg.contains("limit") {
        return Outcome::Limit(msg.lines().next().unwrap_or("").to_string());
    }
    // Any other error from a host function behaves as a trap.
    Outcome::Trap
}

pub fn engine() -> Engine {
    let mut c = Config::new();
    c.wasm_gc(true)
        .wasm_function_references(true)
        .wasm_exceptions(true)
        .wasm_stack_switching(true)
        .wasm_tail_call(true)
        .wasm_multi_memory(true)
        .wasm_memory64(true)
        .wasm_wide_arithmetic(true)
        .wasm_threads(true)
        .shared_memory(true)
        .wasm_custom_page_sizes(true)
        .wasm_extended_const(true)
        .wasm_relaxed_simd(true)
        .relaxed_simd_deterministic(true)
        .consume_fuel(true)
        .max_wasm_stack(1 << 20)
        .collector(Collector::DeferredReferenceCounting);
    Engine::new(&c).unwrap_or_else(|e| {
        eprintln!("engine: {e:?}");
        exit(4)
    })
}

fn run(path: &str) -> i32 {
    let engine = engine();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return 4;
        }
    };
    let module = match Module::new(&engine, &bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("compile: {e:?}");
            return 4;
        }
    };
    let limits = StoreLimitsBuilder::new()
        .memory_size(256 << 20)
        .table_elements(100_000)
        .instances(10)
        .tables(1000)
        .memories(1000)
        .build();
    let host = Host { out: Vec::new(), exports: None, table: None, wasmtag: None, jstag: None, temp_ret0: 0, limits };
    let mut store = Store::new(&engine, host);
    store.limiter(|h| &mut h.limits);
    store.set_fuel(FUEL).unwrap();

    let wasmtag = Tag::new(&mut store, &TagType::new(FuncType::new(&engine, [ValType::I32], []))).unwrap();
    let jstag = Tag::new(&mut store, &TagType::new(FuncType::new(&engine, [ValType::EXTERNREF], []))).unwrap();
    store.data_mut().wasmtag = Some(wasmtag);
    store.data_mut().jstag = Some(jstag);

    let mut imports = Vec::new();
    for imp in module.imports() {
        let ext: Extern = match imp.ty() {
            ExternType::Func(ft) => host_func(&mut store, imp.module(), imp.name(), ft).into(),
            ExternType::Tag(_) if imp.module() == "fuzzing-support" && imp.name() == "wasmtag" => wasmtag.into(),
            ExternType::Tag(_) if imp.module() == "fuzzing-support" && imp.name() == "jstag" => jstag.into(),
            ExternType::Global(gt) => {
                // Synthesized immutable externref globals, payload as in
                // execution-results.h FuzzerImportResolver.
                let payload =
                    imp.module().bytes().chain(imp.name().bytes()).fold(0u32, |p, c| (p + c as u32) % 251);
                let r = ExternRef::new(&mut store, payload).unwrap();
                match Global::new(&mut store, gt, Val::ExternRef(Some(r))) {
                    Ok(g) => g.into(),
                    Err(e) => {
                        eprintln!("unsupported global import {}.{}: {e}", imp.module(), imp.name());
                        return 4;
                    }
                }
            }
            _ => {
                eprintln!("unsupported import {}.{}", imp.module(), imp.name());
                return 4;
            }
        };
        imports.push(ext);
    }

    let instance = match Instance::new(&mut store, &module, &imports) {
        Ok(i) => i,
        Err(e) => {
            if e.downcast_ref::<Trap>().is_none() && !e.is::<ThrownException>() {
                let msg = format!("{e:?}");
                if !(msg.contains("out of memory") || msg.contains("GC heap") || msg.contains("limit")) {
                    eprintln!("instantiate: {msg}");
                    return 4;
                }
            }
            match classify(&mut store, &e) {
                Outcome::Limit(m) => {
                    eprintln!("host limit: {m}");
                    return 3;
                }
                Outcome::Trap => store.data_mut().out.push("[instantiate: trap]".into()),
                Outcome::Exception => store.data_mut().out.push("[instantiate: exception]".into()),
            }
            print_out(&store);
            return 0;
        }
    };

    let exports: Vec<(String, Extern)> =
        instance.exports(&mut store).map(|e| (e.name().to_string(), e.into_extern()))
            // --asyncify appends its control API (asyncify_start_unwind, ...);
            // calling it is not part of the program, and dropping it keeps
            // call-export indices as they were before the pass.
            .filter(|(n, _)| !n.starts_with("asyncify_"))
            .collect();
    store.data_mut().table = exports.iter().find(|(n, _)| n == "table").and_then(|(_, e)| e.clone().into_table());
    store.data_mut().exports = Some(exports.iter().map(|(_, e)| e.clone()).collect());

    for (name, ext) in &exports {
        match ext {
            Extern::Func(f) => {
                store.data_mut().out.push(format!("[fuzz-exec] export {name}"));
                let ty = f.ty(&store);
                if has_cont(&ty) {
                    store.data_mut().out.push("[skip: cont at the host boundary]".into());
                    continue;
                }
                let mut args = Vec::new();
                let mut ok = true;
                for p in ty.params() {
                    match Val::default_for_ty(&p) {
                        Some(v) => args.push(v),
                        None => ok = false,
                    }
                }
                if !ok {
                    store.data_mut().out.push("[trap fuzzer can only send defaultable parameters to exports]".into());
                    continue;
                }
                let mut results: Vec<Val> =
                    ty.results().map(|t| Val::default_for_ty(&t).unwrap_or(Val::I32(0))).collect();
                let r = f.call(&mut store, &args, &mut results);
                match r {
                    Ok(()) => {
                        for v in &results {
                            let s = fmt_val(&mut store, v);
                            store.data_mut().out.push(format!("[fuzz-exec] note result: {name} => {s}"));
                        }
                    }
                    Err(e) => match classify(&mut store, &e) {
                        Outcome::Limit(m) => {
                            eprintln!("host limit: {m}");
                            return 3;
                        }
                        Outcome::Trap => store.data_mut().out.push("[trap]".into()),
                        Outcome::Exception => store.data_mut().out.push("[exception thrown]".into()),
                    },
                }
            }
            Extern::Global(g) => {
                store.data_mut().out.push(format!("[fuzz-exec] export {name}"));
                let s = if is_cont(g.ty(&store).content()) {
                    "contref".to_string()
                } else {
                    let v = g.get(&mut store);
                    fmt_val(&mut store, &v)
                };
                store.data_mut().out.push(format!("[LoggingExternalInterface logging {s}]"));
            }
            _ => {}
        }
    }
    // BYNEXEC_MEMHASH: also print a hash of every exported memory's final
    // contents (wasm-smith modules have no logging imports, so stores are
    // otherwise unobservable).
    if std::env::var_os("BYNEXEC_MEMHASH").is_some() {
        for (name, ext) in &exports {
            let h = |it: &mut dyn Iterator<Item = u8>| {
                let (mut h, mut n) = (0xcbf29ce484222325u64, 0u64);
                for b in it {
                    h = (h ^ b as u64).wrapping_mul(0x100000001b3);
                    n += 1;
                }
                (n, h)
            };
            let r = match ext {
                Extern::Memory(m) => Some(h(&mut m.data(&store).iter().copied())),
                Extern::SharedMemory(m) => {
                    // SAFETY: no other thread runs; plain reads of the bytes.
                    Some(h(&mut m.data().iter().map(|c| unsafe { *c.get() })))
                }
                _ => None,
            };
            if let Some((n, hash)) = r {
                store.data_mut().out.push(format!("[memhash {name:?}] {n} {hash:016x}"));
            }
        }
    }
    print_out(&store);
    0
}

fn print_out(store: &Store<Host>) {
    use std::io::Write;
    let mut o = std::io::stdout().lock();
    for l in &store.data().out {
        let _ = writeln!(o, "{l}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: bynexec <m.wasm>");
        exit(2);
    }
    let path = args[1].clone();
    // Big native stack so deep wasm/host recursion hits the wasm stack limit
    // (a host limit) rather than the thread's guard page.
    let h = std::thread::Builder::new().stack_size(256 << 20).spawn(move || run(&path)).unwrap();
    let code = h.join().unwrap_or(101);
    exit(code);
}
