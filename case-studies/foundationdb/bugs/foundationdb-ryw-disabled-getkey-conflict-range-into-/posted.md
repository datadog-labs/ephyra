# Posted: https://github.com/apple/foundationdb/issues/14478

Filed by alpaylan, 2026-11-09. Text as posted:

---

## `read_your_writes_disable`: spurious conflicts with system-key writes after reading past the last key

```python
import fdb
fdb.api_version(730)
db = fdb.open()

del db[b"":b"\xff"]                 # empty database

tr = db.create_transaction()
tr.options.set_read_your_writes_disable()
print(tr.get_key(fdb.KeySelector.first_greater_or_equal(b"")).wait())
tr[b"a"] = b"1"

other = db.create_transaction()
other.options.set_access_system_keys()
other[b"\xff\x02/a"] = b"x"         # a system key that tr did not read
other.commit().wait()

tr.commit().wait()
print("committed")
```

**Expected behavior**

```
b'\xff'
committed
```

**Actual behavior** (7.3.77 and main 7e7611f82)

```
b'\xff'
Traceback (most recent call last):
  ...
fdb.impl.FDBError: b'Transaction not committed due to conflict with another transaction' (1020)
```
