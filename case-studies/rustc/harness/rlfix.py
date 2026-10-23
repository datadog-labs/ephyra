# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Make rustlantis custom-MIR programs well-defined under the MIR semantics that
-Zmir-move-elimination opts into (RFC 3943), not only under today's semantics.

options.rs: mir_move_elimination "enable the experimental MIR semantics for the move elimination
optimization". RFC 3943 (rust-lang/rfcs#3943, "Precise semantics for local lifetimes"): a local
without storage markers starts **live** but unallocated; only a write to a place without `Deref`
allocates it; "Evaluation of a base local in non-destination place mode | UB if not **allocated**";
"Evaluation of a move operand with no projections | ... | State becomes **live**" (deallocated), and
"Any use of the local that doesn't initialize it, even taking its address, is UB if the local is
unallocated."

rustlantis targets today's semantics and routinely does both: `_1 = addr_of_mut!(_31)` before any
direct write to `_31` (it initialises `_31` only through the pointer), and keeps using `_1` after
`_3.1 = Move(_1)`. Miri (nightly) does not implement the new semantics yet, so it calls these
programs clean while MoveElimination legitimately overlaps `_31` with other locals.

The rewrite keeps behaviour under today's semantics (Miri-clean programs never read the values
written here: a local is written before it is read) and makes the programs valid under both:
  * every non-argument local (and RET) whose address is taken gets a write of one scalar leaf at the
    start of bb0, which allocates it for the whole function (no StorageDead in rustlantis output);
  * bare-local `Move(_N)` / `Move(RET)` operands become copies, so no local is deallocated.
Returns None when some address-taken local has a type the rewrite can't initialise (enums); the
caller then leaves out the -Zmir-move-elimination configs for that program.
"""
import re

_FN = re.compile(r'^(?:pub )?fn (\w+)\((.*)\) -> (.*) \{$')
_LET = re.compile(r'^let (_\d+): (.*);$')
_ADDR = re.compile(r'addr_of(?:_mut)?!\(\s*(_\d+|RET)\b')
_BARE_MOVE = re.compile(r'\bMove\((_\d+|RET)\)')
_INT = re.compile(r'^[iu](8|16|32|64|128|size)$')


def _split_top(s):
    out, cur, d = [], '', 0
    for c in s:
        if c in '([{':
            d += 1
        elif c in ')]}':
            d -= 1
        if c == ',' and d == 0:
            out.append(cur.strip())
            cur = ''
        else:
            cur += c
    if cur.strip():
        out.append(cur.strip())
    return out


def _scalar(t):
    if t.startswith('*'):
        return f'0_usize as {t}'
    if t in ('f32', 'f64'):
        return f'0.0_{t}'
    if t == 'bool':
        return 'false'
    if t == 'char':
        return "'a'"
    if _INT.match(t):
        return f'0_{t}'
    return None


def _leaf(place, t, adts, depth=0):
    """-> (place expression of one scalar leaf of a value of type t at place, value) or None.
    Enums: a field of variant 0 (rustlantis writes enum fields the same way)."""
    t = t.strip()
    z = _scalar(t)
    if z is not None:
        return place, z
    if depth > 8:
        return None
    if t.startswith('[') and t.endswith(']'):
        elem, n = t[1:-1].rsplit(';', 1)
        z = _scalar(elem.strip())
        return (place, f'[{z}; {n.strip()}]') if z is not None else None
    if t.startswith('(') and t.endswith(')'):
        fields = [(f'{place}.{k}', ft) for k, ft in enumerate(_split_top(t[1:-1]))]
    elif t in adts:
        kind, flds = adts[t]
        if kind == 'struct':
            fields = [(f'{place}.{name}', ft) for name, ft in flds]
        else:
            fields = [(f'Field::<{ft}>(Variant({place}, 0), {k})', ft) for k, (_, ft) in enumerate(flds)]
    else:
        return None
    for p, ft in fields:   # writing any field allocates the whole local
        r = _leaf(p, ft, adts, depth + 1)
        if r:
            return r
    return None


def _leaf_init(place, t, adts):
    r = _leaf(place, t, adts)
    if r is None:
        return None
    p, z = r
    return f'place!({p}) = {z};' if 'Variant(' in p else f'{p} = {z};'


def _structs(lines):
    """-> {name: ('struct', [(field, ty)]) | ('enum', fields of Variant0)}"""
    adts, cur = {}, None
    for line in lines:
        m = re.match(r'^pub (struct|enum) (\w+) \{$', line)
        if m:
            cur = []
            adts[m.group(2)] = (m.group(1), cur)
            continue
        if cur is None:
            continue
        m = re.match(r'^(\w+): (.*),$', line)
        if m:
            cur.append((m.group(1), m.group(2)))
        elif line == 'Variant0{':
            continue
        else:
            cur = None
    return adts


def new_semantics(text):
    lines = text.split('\n')
    structs = _structs(lines)
    out, i, ok = [], 0, True
    while i < len(lines):
        line = lines[i]
        m = _FN.match(line) if i and lines[i - 1].lstrip().startswith('#[custom_mir') else None
        if not m:
            out.append(line)
            i += 1
            continue
        # one custom-MIR function: header, `mir! {`, `type RET = ..;`, lets, `{` (bb0), blocks
        end = next((k for k in range(i + 1, len(lines)) if lines[k].lstrip().startswith('#[custom_mir')), len(lines))
        body = lines[i:end]
        types = {}
        for b in body:
            lm = _LET.match(b)
            if lm:
                types[lm.group(1)] = lm.group(2)
            rm = re.match(r'^type RET = (.*);$', b)
            if rm:
                types['RET'] = rm.group(1)
        inits = []
        for v in sorted(set(_ADDR.findall('\n'.join(body)))):
            if v not in types:
                continue   # argument: allocated on entry
            init = _leaf_init(v, types[v], structs)
            if init is None:
                ok = False
                break
            inits.append(init)
        bb0 = next(k for k, b in enumerate(body) if b == '{')
        body = body[:bb0 + 1] + inits + body[bb0 + 1:]
        out.extend(_BARE_MOVE.sub(r'\1', b) for b in body)
        i = end
    return '\n'.join(out) if ok else None
