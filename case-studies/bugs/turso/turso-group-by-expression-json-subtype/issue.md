Filed as #9966 (https://github.com/tursodatabase/turso/issues/9966) (line 1 corrected 2026-10-21).
Title: JSON subtype lost when a result column contains an expression from GROUP BY (follow-up to #5755)

#5755 (JSON subtype lost through GROUP BY) is closed, and its repros pass now. The subtype is still lost in one case: a result column that contains the GROUP BY expression itself. The JSON value then gets quoted as a string.

```sql
CREATE TABLE t(a, b);
INSERT INTO t VALUES (1, 'x'), (2, 'y');
SELECT json_array(json_object('k', b)) FROM t GROUP BY json_object('k', b);
SELECT json_quote(json_quote(b)) FROM t GROUP BY json_quote(b);
SELECT json_array(json(a || '')) FROM t GROUP BY json(a || '');
SELECT json_array(json_object('k', b)) FROM t GROUP BY 1;
```

Turso (`tursodb -q -m list`):

```
["{\"k\":\"x\"}"]
["{\"k\":\"y\"}"]
"\"x\""
"\"y\""
["1"]
["2"]
[{"k":"x"}]
[{"k":"y"}]
```

SQLite 3.50.4 (trunk is the same):

```
[{"k":"x"}]
[{"k":"y"}]
"x"
"y"
[1]
[2]
[{"k":"x"}]
[{"k":"y"}]
```

Expected: the SQLite output. The last query (`GROUP BY 1`) already works. HAVING doesn't matter.

Likely cause: in the sorter path of the GROUP BY output (`core/translate/group_by.rs` ~908), each GROUP BY expression is read back from the sorter record with `emit_column_or_rowid` and cached with `cache_scalar_expr_reg`. A larger expression in the result list then uses that cached register. The sorter record doesn't keep the subtype, so the value is plain text again. SQLite computes the expression again from the stored columns instead.

Tested on Turso main abf885b2 (also on f8549ca2, so not a regression).
