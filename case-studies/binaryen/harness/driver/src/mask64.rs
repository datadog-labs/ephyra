// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Masks every 64-bit address, size and delta operand of memory and table
//! instructions in a module printed by binaryen (`wasm-opt -S`) to its low 32 bits:
//! `x` becomes `(i64.extend_i32_u (i32.wrap_i64 x))`. The module stays 64-bit;
//! it now behaves the way `--memory64-lowering` (which wraps each such operand
//! to i32) treats it, so a run of the masked module that matches the lowered
//! one shows that a difference came only from operands >= 2^32.

enum Node {
    Atom(usize, usize),
    List(usize, usize, Vec<Node>),
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    loop {
        while i < s.len() && s[i].is_ascii_whitespace() {
            i += 1;
        }
        if s[i..].starts_with(b";;") {
            while i < s.len() && s[i] != b'\n' {
                i += 1;
            }
        } else if s[i..].starts_with(b"(;") {
            let mut depth = 0;
            while i < s.len() {
                if s[i..].starts_with(b"(;") {
                    depth += 1;
                    i += 2;
                } else if s[i..].starts_with(b";)") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else {
            return i;
        }
    }
}

fn skip_string(s: &[u8], mut i: usize) -> Option<usize> {
    // s[i] == '"'
    i += 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn parse(s: &[u8], i: usize) -> Option<(Node, usize)> {
    let i = skip_ws(s, i);
    if i >= s.len() {
        return None;
    }
    if s[i] == b'(' {
        let start = i;
        let mut j = i + 1;
        let mut kids = Vec::new();
        loop {
            j = skip_ws(s, j);
            if j >= s.len() {
                return None;
            }
            if s[j] == b')' {
                return Some((Node::List(start, j + 1, kids), j + 1));
            }
            let (n, k) = parse(s, j)?;
            kids.push(n);
            j = k;
        }
    }
    let start = i;
    let mut j = i;
    while j < s.len() && !s[j].is_ascii_whitespace() && s[j] != b'(' && s[j] != b')' {
        if s[j] == b'"' {
            j = skip_string(s, j)?;
        } else {
            j += 1;
        }
    }
    if j == start {
        return None;
    }
    Some((Node::Atom(start, j), j))
}

fn atom<'a>(s: &'a str, n: &Node) -> Option<&'a str> {
    match n {
        Node::Atom(a, b) => Some(&s[*a..*b]),
        _ => None,
    }
}

fn head<'a>(s: &'a str, n: &Node) -> Option<&'a str> {
    match n {
        Node::List(_, _, k) => k.first().and_then(|h| atom(s, h)),
        _ => None,
    }
}

/// Names of the memories and tables in declaration order, and whether each
/// one is 64-bit.
fn decls(s: &str, n: &Node, mems: &mut Vec<(String, bool)>, tabs: &mut Vec<(String, bool)>) {
    let Node::List(_, _, k) = n else { return };
    match head(s, n) {
        Some("import") => k.iter().for_each(|c| decls(s, c, mems, tabs)),
        Some(h @ ("memory" | "table")) => {
            let name = k.get(1).and_then(|c| atom(s, c)).filter(|a| a.starts_with('$')).unwrap_or("").to_string();
            let is64 = k.iter().skip(1).take(3).any(|c| atom(s, c) == Some("i64"));
            if h == "memory" { mems.push((name, is64)) } else { tabs.push((name, is64)) }
        }
        _ => {}
    }
}

struct Ctx<'a> {
    s: &'a str,
    mems: Vec<(String, bool)>,
    tabs: Vec<(String, bool)>,
    edits: Vec<(usize, usize)>,
    // Literal replacements (start, end, text).
    lits: Vec<(usize, usize, String)>,
}

impl Ctx<'_> {
    fn is64(list: &[(String, bool)], name: Option<&str>) -> bool {
        match name {
            // Not one of `list` (e.g. the segment of memory.init): the default.
            Some(n) if list.iter().any(|(m, _)| m == n) => list.iter().any(|(m, b)| m == n && *b),
            _ => list.first().map_or(false, |(_, b)| *b),
        }
    }

    fn walk(&mut self, n: &Node) {
        let Node::List(_, _, k) = n else { return };
        k.iter().for_each(|c| self.walk(c));
        let Some(h) = head(self.s, n) else { return };
        // `$name` immediates following the opcode.
        let names: Vec<&str> = k.iter().skip(1).map_while(|c| atom(self.s, c)).filter(|a| a.starts_with('$')).collect();
        let ops: Vec<&Node> = k
            .iter()
            .skip(1)
            .filter(|c| matches!(c, Node::List(..)) && !matches!(head(self.s, c), Some("type" | "param" | "result")))
            .collect();
        let (m, t) = (&self.mems, &self.tabs);
        let n0 = names.first().copied();
        let n1 = names.get(1).copied();
        let mut mask: Vec<usize> = Vec::new();
        // Active segment offsets: the pass truncates a constant i64 offset of
        // a 64-bit memory/table to u32 (visitDataSegment/visitElementSegment).
        // A constant expression cannot hold i32.wrap_i64, so mask the literal.
        if h == "data" || h == "elem" {
            let target = k.iter().skip(1).find_map(|c| match head(self.s, c) {
                Some("memory" | "table") => match c {
                    Node::List(_, _, ck) => ck.get(1).and_then(|x| atom(self.s, x)),
                    _ => None,
                },
                _ => None,
            });
            let is64 = if h == "data" { Self::is64(m, target) } else { Self::is64(t, target) };
            let off = k.iter().skip(1).find(|c| matches!(head(self.s, c), Some("offset" | "i64.const")));
            if let (true, Some(Node::List(_, _, ok))) = (is64, off) {
                // `(i64.const N)` or `(offset (i64.const N))`.
                let lit = if head(self.s, off.unwrap()) == Some("offset") {
                    match ok.get(1) {
                        Some(c @ Node::List(_, _, ck)) if head(self.s, c) == Some("i64.const") => ck.get(1),
                        _ => None,
                    }
                } else {
                    ok.get(1)
                };
                if let Some(Node::Atom(a, b)) = lit {
                    let v = &self.s[*a..*b];
                    let n = v.parse::<i64>().ok().or_else(|| v.parse::<u64>().ok().map(|u| u as i64));
                    if let Some(n) = n {
                        self.lits.push((*a, *b, ((n as u64) & 0xffff_ffff).to_string()));
                    }
                }
            }
            return;
        }
        match h {
            _ if h.starts_with("struct.") || h.starts_with("array.") => {}
            "memory.grow" | "memory.init" if Self::is64(m, n0) => mask.push(0),
            "memory.fill" if Self::is64(m, n0) => mask.extend([0, 2]),
            "memory.copy" => {
                let (d, sr) = (Self::is64(m, n0), Self::is64(m, n1.or(n0)));
                if d { mask.push(0) }
                if sr { mask.push(1) }
                if d && sr { mask.push(2) }
            }
            "table.grow" if Self::is64(t, n0) => mask.push(1),
            "table.fill" if Self::is64(t, n0) => mask.extend([0, 2]),
            "table.init" | "table.get" | "table.set" if Self::is64(t, n0) => mask.push(0),
            "table.copy" => {
                let (d, sr) = (Self::is64(t, n0), Self::is64(t, n1.or(n0)));
                if d { mask.push(0) }
                if sr { mask.push(1) }
                if d && sr { mask.push(2) }
            }
            "call_indirect" | "return_call_indirect" if Self::is64(t, n0) && !ops.is_empty() => mask.push(ops.len() - 1),
            _ if (h.contains(".load") || h.contains(".store") || h.contains(".atomic.rmw") || h.starts_with("memory.atomic."))
                && Self::is64(m, n0) => mask.push(0),
            _ => {}
        }
        for i in mask {
            if let Some(Node::List(a, b, _)) = ops.get(i) {
                self.edits.push((*a, *b));
            }
        }
    }
}

/// The masked module text, or None when `wat` could not be parsed.
pub fn mask(wat: &str) -> Option<String> {
    let (root, _) = parse(wat.as_bytes(), 0)?;
    let (mut mems, mut tabs) = (Vec::new(), Vec::new());
    if let Node::List(_, _, k) = &root {
        k.iter().for_each(|c| decls(wat, c, &mut mems, &mut tabs));
    }
    let mut cx = Ctx { s: wat, mems, tabs, edits: Vec::new(), lits: Vec::new() };
    cx.walk(&root);
    let mut ins: Vec<(usize, &str)> = Vec::new();
    for (a, b) in &cx.edits {
        ins.push((*a, "(i64.extend_i32_u (i32.wrap_i64 "));
        ins.push((*b, "))"));
    }
    // Segment offset literals: never inside an instruction operand edit, so
    // splice them separately (start, end) -> text, back to front.
    let mut wat = std::borrow::Cow::Borrowed(wat);
    if !cx.lits.is_empty() {
        let shift: Vec<(usize, isize)> = cx.lits.iter().map(|(a, b, t)| (*a, t.len() as isize - (b - a) as isize)).collect();
        let mut w = wat.to_string();
        let mut lits = cx.lits.clone();
        lits.sort_by_key(|(a, _, _)| std::cmp::Reverse(*a));
        for (a, b, t) in &lits {
            w.replace_range(*a..*b, t);
        }
        // Move the instruction edits past the replaced literals.
        for (p, _) in ins.iter_mut() {
            let d: isize = shift.iter().filter(|(a, _)| *a < *p).map(|(_, d)| d).sum();
            *p = (*p as isize + d) as usize;
        }
        wat = std::borrow::Cow::Owned(w);
    }
    let wat: &str = &wat;
    // Closers before openers at the same position (a masked operand ending
    // where another begins).
    ins.sort_by_key(|(p, t)| (*p, t.starts_with('(')));
    let mut out = String::with_capacity(wat.len() + ins.len() * 32);
    let mut last = 0;
    for (p, t) in ins {
        out.push_str(&wat[last..p]);
        out.push_str(t);
        last = p;
    }
    out.push_str(&wat[last..]);
    Some(out)
}
