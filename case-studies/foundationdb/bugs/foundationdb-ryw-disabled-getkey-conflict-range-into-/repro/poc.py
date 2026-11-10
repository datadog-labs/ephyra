# usage: poc.py <cluster-file> [api_version]
# Exit 1 = bug reproduces, 0 = correct.
import sys
import fdb
fdb.api_version(int(sys.argv[2]) if len(sys.argv) > 2 else 730)
db = fdb.open(sys.argv[1])

def reset(db):
    tr = db.create_transaction()
    tr.options.set_access_system_keys()
    tr.clear_range(b"", b"\xff")
    tr.clear(b"\xff\x02/aaa")
    tr.commit().wait()

def run(ryw_disabled):
    reset(db)
    t1 = db.create_transaction()
    if ryw_disabled:
        t1.options.set_read_your_writes_disable()
    k = t1.get_key(fdb.KeySelector.first_greater_than(b"")).wait()   # empty database: returns b'\xff'
    t1[b"a"] = b"1"

    t2 = db.create_transaction()                 # concurrent write to a system key, not read by t1
    t2.options.set_access_system_keys()
    t2[b"\xff\x02/aaa"] = b"x"
    t2.commit().wait()

    try:
        t1.commit().wait()
        res = "committed"
    except fdb.FDBError as e:
        res = "error %d (%s)" % (e.code, e.description)
    print("read_your_writes_disable=%s  get_key -> %r  commit -> %s" % (ryw_disabled, k, res))
    return res

on = run(False)
off = run(True)
reset(db)
if on == "committed" and off != "committed":
    print("BUG: the read_your_writes_disable transaction conflicts with a write to a key it did not read")
    sys.exit(1)
print("OK")
sys.exit(0)
