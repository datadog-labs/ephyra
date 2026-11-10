import sys, fdb
fdb.api_version(int(sys.argv[2]))
db = fdb.open(sys.argv[1])
KS = fdb.KeySelector
def fresh(keys):
    del db[b"":b"\xff"]
    for k, v in keys: db[k] = v
def run(name, keys, pre_get, op, expect):
    fresh(keys)
    tr = db.create_transaction()
    tr.compare_and_clear(b"b", b"x")
    if pre_get: tr[b"b"].present()
    got = op(tr)
    print(f"{name:55s} pre_get={pre_get!s:5s} got={got!r:22s} expect={expect!r:22s} {'BUG' if got != expect else 'ok'}")
for pre in (False, True):
    run("empty db: get_key(last_less_than c)", [], pre, lambda t: t.get_key(KS.last_less_than(b"c")).wait(), b"")
    run("{a}: get_key(last_less_than c)", [(b"a", b"v")], pre, lambda t: t.get_key(KS.last_less_than(b"c")).wait(), b"a")
    run("{a}: get_range(last_less_than c, \\xff)", [(b"a", b"v")], pre, lambda t: [tuple(kv) for kv in t.get_range(KS.last_less_than(b"c"), b"\xff")], [(b"a", b"v")])
    run("{c}: get_key(first_greater_than a)", [(b"c", b"v")], pre, lambda t: t.get_key(KS.first_greater_than(b"a")).wait(), b"c")
    run("{c}: get_range(first_greater_than a, \\xff)", [(b"c", b"v")], pre, lambda t: [tuple(kv) for kv in t.get_range(KS.first_greater_than(b"a"), b"\xff")], [(b"c", b"v")])
    run("{a,c}: get_range(a, \\xff, limit=2)", [(b"a", b"v"), (b"c", b"v")], pre, lambda t: [tuple(kv) for kv in t.get_range(b"a", b"\xff", limit=2)], [(b"a", b"v"), (b"c", b"v")])
    run("{a,c}: get_key(first_greater_or_equal a + 1)", [(b"a", b"v"), (b"c", b"v")], pre, lambda t: t.get_key(KS.first_greater_or_equal(b"a") + 1).wait(), b"c")
