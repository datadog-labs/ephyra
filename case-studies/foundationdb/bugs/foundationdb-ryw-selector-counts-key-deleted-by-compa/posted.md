# Posted: https://github.com/apple/foundationdb/issues/14477

Filed by alpaylan, 2026-11-09 04:33 UTC. Text as posted:

---

## Read-your-writes: `get_range` with a key selector counts a key that `compare_and_clear` left absent

**Snippet**

```python
import fdb
fdb.api_version(730)
db = fdb.open()

del db[b"":b"\xff"]
db[b"a"] = b"v"

tr = db.create_transaction()
tr.compare_and_clear(b"b", b"x")  # pick a key between 'a' and 'c'
print([tuple(kv) for kv in tr.get_range(fdb.KeySelector.last_less_than(b"c"), b"\xff")])
```

**Expected behavior**

```
[(b'a', b'v')]
```

**Actual behavior** (7.3.77 and main 91a4528298c0)

```
[]
```

With `read_your_writes_disable`, or after commit, the results are correct.

