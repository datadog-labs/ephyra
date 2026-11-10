#!/usr/bin/env python3
"""compare_and_clear on an absent key shifts key selectors in the same transaction.

usage: poc.py <cluster_file> [api_version]     (exit 1 = bug shows, 0 = correct)
"""
import sys
import fdb

fdb.api_version(int(sys.argv[2]) if len(sys.argv) > 2 else 730)
db = fdb.open(sys.argv[1])

del db[b"":b"\xff"]
db[b"a"] = b"v"                       # database: {a: v}, key b does not exist

tr = db.create_transaction()
tr.compare_and_clear(b"b", b"x")      # b is absent, so it stays absent
sel = fdb.KeySelector.last_less_than(b"c")
get_b = tr[b"b"].present()
get_key = tr.get_key(fdb.KeySelector(b"c", False, -1)).wait()  # second-to-last key before c
get_range = list(tr.get_range(sel, b"\xff"))
tr.cancel()

print("get(b) present:", get_b)
print("get_key(KeySelector(c, False, -1)):", get_key)
print("get_range(last_less_than(c), \\xff):", [(kv.key, kv.value) for kv in get_range])

ok = (not get_b and get_key == b"" and [(kv.key, kv.value) for kv in get_range] == [(b"a", b"v")])
print("OK" if ok else "BUG: selector counted the compare-and-cleared key b")
sys.exit(0 if ok else 1)
