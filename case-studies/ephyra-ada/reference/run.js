// Runs cases on whatwg-url, the standard's reference implementation: one
// JSON case per line on stdin ({input, base, sets: [[setter, value]]}), one
// JSON transcript per line on stdout ({can_parse, steps: [{ok, url}]}).
// Setters other than href report nothing and count as succeeding; an href
// setter that throws has failed and leaves the URL unchanged.
"use strict";
const { URL, URLSearchParams } = require("whatwg-url");
const readline = require("readline");
const { URLPattern } = require("urlpattern-polyfill/urlpattern");
const tr46 = require("tr46");
const IDNA_OPTS = { checkHyphens: false, checkBidi: true, checkJoiners: true, useSTD3ASCIIRules: false, transitionalProcessing: false, verifyDNSLength: false };

const GETTERS = ["href", "origin", "protocol", "username", "password", "host", "hostname", "port",
  "pathname", "search", "hash"];

function parts(u) {
  const out = {};
  for (const g of GETTERS) out[g] = u[g];
  return out;
}

function run(c) {
  const base = c.base === null ? undefined : c.base;
  const can_parse = URL.canParse(c.input, base);
  let u;
  try {
    u = new URL(c.input, base);
  } catch {
    return { can_parse, steps: [{ ok: false, url: null }] };
  }
  const steps = [{ ok: true, url: parts(u) }];
  for (const [setter, value] of c.sets) {
    let ok = true;
    try {
      u[setter] = value;
    } catch {
      ok = false;
    }
    steps.push({ ok, url: parts(u) });
  }
  return { can_parse, steps };
}

// A URLSearchParams case ({params, ops: [[op, key, value]]}): after the
// constructor and each operation, its result, size, toString() and entries.
function runParams(c) {
  const p = new URLSearchParams(c.params);
  const steps = [];
  const step = (result) => steps.push({ result, size: p.size, serialized: p.toString(),
    entries: [...p].flat() });
  step([]);
  const live = p.entries();
  for (const [op, k, v] of c.ops) {
    switch (op) {
      case "keys": step([...p.keys()]); break;
      case "values": step([...p.values()]); break;
      case "entries": step([...p.entries()].flat()); break;
      case "next": { const n = live.next(); step(n.done ? [] : n.value); break; }
      case "append": p.append(k, v); step([]); break;
      case "delete": p.delete(k); step([]); break;
      case "deleteValue": p.delete(k, v); step([]); break;
      case "set": p.set(k, v); step([]); break;
      case "sort": p.sort(); step([]); break;
      case "get": { const got = p.get(k); step(got === null ? [] : [got]); break; }
      case "getAll": step(p.getAll(k)); break;
      case "has": step([String(p.has(k))]); break;
      case "hasValue": step([String(p.has(k, v))]); break;
    }
  }
  return steps;
}

// A URLPattern case ({pattern, base, ignoreCase, inputs: [[url, base]]}):
// whether it compiles, its component strings, and per input test and exec
// (0 false or no match, 1 true or a match, 2 an error) with, for a match,
// each component's input and its groups as sorted names and values ("\0"
// for undefined).
const COMPONENTS = ["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"];
function runPattern(c) {
  let p;
  try {
    p = c.base === null ? new URLPattern(c.pattern, { ignoreCase: c.ignoreCase })
                        : new URLPattern(c.pattern, c.base, { ignoreCase: c.ignoreCase });
  } catch {
    return { ok: false };
  }
  const inputs = c.inputs.map(([input, base]) => {
    const args = base === null ? [input] : [input, base];
    let test, exec, groups = [];
    try { test = p.test(...args) ? 1 : 0; } catch { test = 2; }
    try {
      const m = p.exec(...args);
      exec = m ? 1 : 0;
      if (m) groups = COMPONENTS.map((k) => [m[k].input, Object.keys(m[k].groups).sort()
        .flatMap((n) => [n, m[k].groups[n] === undefined ? "\0" : m[k].groups[n]])]);
    } catch { exec = 2; }
    return { test, exec, groups };
  });
  return { ok: true, components: COMPONENTS.map((k) => p[k]), inputs };
}

// URLPatternInit inputs ({initInputs: [init]}) through new URLPattern({}):
// per input, status (0 no match, 1 match, 2 error) and the component inputs.
function runInitInputs(c) {
  const p = new URLPattern({});
  return c.initInputs.map((init) => {
    try {
      const m = p.exec(init);
      return m ? { status: 1, parts: COMPONENTS.map((k) => m[k].input) } : { status: 0, parts: [] };
    } catch {
      return { status: 2, parts: [] };
    }
  });
}

// Domains through tr46 with ada's flags: toASCII (null on failure) and
// toUnicode's domain (its error flag is ignored, as ada's to_unicode has none).
function runIdna(c) {
  return c.idna.map((d) => {
    let ascii = null;
    try { ascii = tr46.toASCII(d, IDNA_OPTS); } catch { ascii = null; }
    let unicode = "";
    try { unicode = tr46.toUnicode(d, IDNA_OPTS).domain; } catch { unicode = ""; }
    return { asciiOk: ascii !== null, ascii: ascii === null ? "" : ascii, unicode };
  });
}

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  const c = JSON.parse(line);
  const answer = "idna" in c ? runIdna(c)
    : "initInputs" in c ? runInitInputs(c)
    : "pattern" in c ? runPattern(c) : "params" in c ? runParams(c) : run(c);
  process.stdout.write(JSON.stringify(answer) + "\n");
});
