Filed as #10342 (https://github.com/tursodatabase/turso/issues/10342) on 2026-10-21.
Title: json_each/json_tree declare column types (fullkey TEXT, key ANY, ...), so comparisons with numbers use the wrong affinity

```sql
SELECT count(*) FROM json_each('{"a":1}') WHERE fullkey < 0;
SELECT fullkey < 0, path < 0 FROM json_each('{"a":1}');
CREATE TABLE t AS SELECT key, value, type, fullkey FROM json_each('{"a":1}');
SELECT sql FROM sqlite_schema WHERE name = 't';
```

Turso:
```
1
1|1
CREATE TABLE t("key" NUM,value NUM,type TEXT,fullkey TEXT)
```
SQLite 3.50.4 (and trunk):
```
0
0|0
CREATE TABLE t(
  "key",
  value,
  type,
  fullkey
)
```

Expected: in SQLite the json_each/json_tree columns have no declared type (`pragma_table_xinfo('json_each')` shows an empty `type` for every column), so they have no affinity. A text value such as `'$.a'` is then always greater than a number. Turso declares `fullkey TEXT` and `path TEXT`, so `fullkey < 0` applies TEXT affinity to `0` and compares `'$.a' < '0'` as strings, which is true. The declared types also show up in `CREATE TABLE ... AS SELECT`. `key ANY` and `value ANY` (NUMERIC affinity), `type TEXT`, `id INTEGER`, `parent INTEGER` and the hidden `json JSON` / `root TEXT` differ from SQLite in the same way.

Likely cause: `JsonVirtualTable::sql()` in `core/json/vtab.rs` declares types on the columns. SQLite's schema is `CREATE TABLE x(key,value,type,atom,id,parent,fullkey,path,json HIDDEN,root HIDDEN)`.

Tested on Turso main cbd5ac4e.
