//! URLPatternInit inputs, processed by `exec` against the all-wildcard
//! pattern `{}`: the component inputs a match reports are the processed init
//! ("process a URLPatternInit" with type "url": canonicalization, and
//! components inherited from `baseURL`). That is independent of the regex
//! engine, so ada can be compared with urlpattern-polyfill and Deno's
//! `urlpattern` directly. [`init_cases`] draws the inputs.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};
use serde_json::{json, Value};

use crate::{gen, params::text, pattern::COMPONENTS};

/// The init fields in `URLPatternInit` order, `baseURL` last; absent `None`.
pub type InitInput = [Option<String>; 9];

/// Per input: 0 no match, 1 a match, 2 an error; and for a match the eight
/// component inputs.
pub type InitResult = (u8, Vec<String>);

const NAMES: [&str; 9] = [
    "protocol", "username", "password", "hostname", "port", "pathname", "search", "hash", "baseURL",
];

extern "C" {
    fn ea_init_inputs(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_init_test(bytes: *const u8, len: usize, out: *mut *mut u8) -> usize;
    fn ea_free(p: *mut u8);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn encode(inputs: &[InitInput]) -> Vec<u8> {
    let mut bytes = (inputs.len() as u32).to_le_bytes().to_vec();
    for input in inputs {
        for f in input {
            bytes.push(u8::from(f.is_some()));
            put_str(&mut bytes, f.as_deref().unwrap_or(""));
        }
    }
    bytes
}

/// Calls a shim runner on `bytes`: the transcript it returns.
fn call(f: unsafe extern "C" fn(*const u8, usize, *mut *mut u8) -> usize, bytes: &[u8]) -> Vec<u8> {
    let mut out = std::ptr::null_mut();
    // SAFETY: `bytes` is live for the call; the shim writes a malloc'd
    // buffer of the returned length to `out`, copied and then freed.
    crate::ada::locked(|| unsafe {
        let len = f(bytes.as_ptr(), bytes.len(), &mut out);
        let t = std::slice::from_raw_parts(out, len).to_vec();
        ea_free(out);
        t
    })
}

/// Per input, `test` and `exec` of `{}` in ada (0 false or no match, 1 true
/// or a match, 2 an error): `[test(init), exec(init), test(init, base),
/// exec(init, base)]`, the base a URL string.
pub fn ada_test(inputs: &[InitInput]) -> Vec<[u8; 4]> {
    call(ea_init_test, &encode(inputs)).chunks(4).map(|c| c.try_into().unwrap()).collect()
}

/// The results of `inputs` in ada.
pub fn ada(inputs: &[InitInput]) -> Vec<InitResult> {
    let t = call(ea_init_inputs, &encode(inputs));
    let mut r = &t[..];
    inputs
        .iter()
        .map(|_| {
            let status = r[0];
            r = &r[1..];
            let mut parts = Vec::new();
            if status == 1 {
                for _ in 0..8 {
                    let n = u32::from_le_bytes(r[..4].try_into().unwrap()) as usize;
                    parts.push(text(&r[4..4 + n]));
                    r = &r[4 + n..];
                }
            }
            (status, parts)
        })
        .collect()
}

/// The results of `inputs` in urlpattern-polyfill.
pub fn reference(inputs: &[InitInput]) -> Vec<InitResult> {
    let objects: Vec<Value> = inputs
        .iter()
        .map(|input| {
            let mut o = serde_json::Map::new();
            for (name, f) in NAMES.iter().zip(input) {
                if let Some(v) = f {
                    o.insert((*name).to_owned(), json!(v));
                }
            }
            Value::Object(o)
        })
        .collect();
    let v = crate::reference::request(&json!({ "initInputs": objects }));
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let status = r["status"].as_u64().unwrap() as u8;
            let parts = r["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect();
            (status, parts)
        })
        .collect()
}

/// The results of `inputs` in Deno's `urlpattern` crate, or `None` if it
/// panics (it does on some inputs; a bug in that crate, not ada's). A
/// `baseURL` that does not parse gives no match, as the spec's match does.
pub fn deno(inputs: &[InitInput]) -> Option<Vec<InitResult>> {
    use urlpattern::{UrlPattern, UrlPatternInit, UrlPatternMatchInput, UrlPatternOptions};
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let p =
            <UrlPattern>::parse(UrlPatternInit::default(), UrlPatternOptions::default()).unwrap();
        inputs
            .iter()
            .map(|f| {
                let base_url = match &f[8] {
                    Some(b) => match url::Url::parse(b) {
                        Ok(u) => Some(u),
                        Err(_) => return (0, Vec::new()),
                    },
                    None => None,
                };
                let init = UrlPatternInit {
                    protocol: f[0].clone(),
                    username: f[1].clone(),
                    password: f[2].clone(),
                    hostname: f[3].clone(),
                    port: f[4].clone(),
                    pathname: f[5].clone(),
                    search: f[6].clone(),
                    hash: f[7].clone(),
                    base_url,
                };
                match p.exec(UrlPatternMatchInput::Init(init)) {
                    Ok(Some(m)) => (
                        1,
                        [
                            m.protocol, m.username, m.password, m.hostname, m.port, m.pathname,
                            m.search, m.hash,
                        ]
                        .into_iter()
                        .map(|c| c.input)
                        .collect(),
                    ),
                    Ok(None) => (0, Vec::new()),
                    Err(_) => (2, Vec::new()),
                }
            })
            .collect()
    }))
    .ok()
}

/// One init input: each component present now and then, drawn from the URL
/// grammar for that component (with and without its delimiter), and a
/// `baseURL` from the absolute-URL grammar.
fn input(g: &mut Source) -> InitInput {
    let mut f: InitInput = Default::default();
    let on = |g: &mut Source| g.bool_with(0.4);
    if on(g) {
        let s = gen::scheme(g);
        f[0] = Some(if g.bool() { format!("{s}:") } else { s });
    }
    if on(g) {
        f[1] = Some(gen::text(g, 4));
    }
    if g.bool_with(0.2) {
        f[2] = Some(gen::text(g, 4));
    }
    if on(g) {
        f[3] = Some(gen::host(g));
    }
    if g.bool_with(0.3) {
        f[4] = Some(gen::port(g));
    }
    if on(g) {
        let p = gen::path(g);
        f[5] = Some(if g.bool_with(0.3) { p.trim_start_matches('/').to_owned() } else { p });
    }
    if g.bool_with(0.3) {
        let q = gen::query(g);
        f[6] = Some(if g.bool() { format!("?{q}") } else { q });
    }
    if g.bool_with(0.3) {
        let h = gen::text(g, 4);
        f[7] = Some(if g.bool() { format!("#{h}") } else { h });
    }
    if on(g) {
        f[8] = Some(gen::absolute(g));
    }
    f
}

/// Lists of init inputs. Shrinking drops inputs, then fields, then
/// characters; mutation redraws an input or one of its fields.
pub struct InitCases;

impl Gen for InitCases {
    type Value = Vec<InitInput>;

    fn generate(&self, g: &mut Source) -> Vec<InitInput> {
        (0..1 + gen::few(g, 3)).map(|_| input(g)).collect()
    }

    fn shrink(
        &self,
        v: &Vec<InitInput>,
        visit: &mut dyn FnMut(Vec<InitInput>) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if v.len() > 1 {
            for k in (0..v.len()).rev() {
                let mut out = v.clone();
                out.remove(k);
                visit(out)?;
            }
        }
        for k in 0..v.len() {
            for i in 0..9 {
                if v[k][i].is_some() {
                    let mut out = v.clone();
                    out[k][i] = None;
                    visit(out)?;
                }
            }
        }
        for k in 0..v.len() {
            for i in 0..9 {
                if let Some(s) = &v[k][i] {
                    gen::deletions(s, &mut |t| {
                        let mut out = v.clone();
                        out[k][i] = Some(t);
                        visit(out)
                    })?;
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, v: &Vec<InitInput>, g: &mut Source) -> Vec<InitInput> {
        let mut out = v.clone();
        let k = g.index(out.len());
        if g.bool() {
            out[k] = input(g);
        } else {
            let fresh = input(g);
            let i = g.index(9);
            out[k][i] = fresh[i].clone();
        }
        out
    }
}

/// Numbers about a list of init inputs: how many, and how many fields and
/// base URLs they set.
// ephyra's metrics take `&T` for the generated type, here a `Vec`.
#[allow(clippy::ptr_arg)]
pub fn init_metrics(v: &Vec<InitInput>, _size: usize) -> Vec<(&'static str, f64)> {
    vec![
        ("inputs", v.len() as f64),
        (
            "fields",
            v.iter().map(|f| f.iter().filter(|x| x.is_some()).count()).sum::<usize>() as f64,
        ),
        ("with baseURL", v.iter().filter(|f| f[8].is_some()).count() as f64),
    ]
}

/// The generator of URLPatternInit input lists.
#[ephyra::generator(metrics = init_metrics)]
pub fn init_cases() -> impl Gen<Value = Vec<InitInput>> {
    InitCases
}

/// The component names, for messages.
pub fn component_name(k: usize) -> &'static str {
    COMPONENTS[k]
}
