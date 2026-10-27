# group_concat()/string_agg() over a sliding window: "Internal error: GroupConcat xInverse removes N bytes from M" after an empty string, when the next value starts with a multi-byte character

Posted as https://github.com/tursodatabase/turso/issues/10906 on 2026-10-27. The text below is the issue as posted.

---

```sql
CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT);
INSERT INTO t VALUES (1,'a'),(2,''),(3,'éa'),(4,'b');
SELECT a, quote(group_concat(b) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t;
SELECT a, quote(string_agg(b, '-') OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
1|'a,'
2|'éa'
Runtime error: Internal error: GroupConcat xInverse removes 1 bytes from 3
1|'a-'
2|'éa'
Runtime error: Internal error: GroupConcat xInverse removes 1 bytes from 3
```

SQLite trunk (3.55.0, 8de43901c7):
```
1|'a,'
2|',éa'
3|'éa,b'
4|'b'
1|'a-'
2|'-éa'
3|'éa-b'
4|'b'
```

SQLite 3.50.4 gives `2|'éa'`, then `3|'<A9>a,b'` (the second byte of `é` alone) and `4|''`. That was a SQLite bug, fixed on trunk on 2026-10-16 by check-in 2f794cf18a ("Fix a problem handling zero-length strings in some cases in group_concat() when it is used as a window function"; test/windowE.test).

Expected: the SQLite trunk output. Row 2's frame is `('', 'éa')`, so the result is `',éa'`. Row 3's frame is `('éa', 'b')`.

Likely cause: the fix for #10352 (#10671) follows SQLite 3.50.4's `groupConcatInverse`. In `inverse_agg_payload` (core/vdbe/execute.rs, `GroupConcat | StringAgg` arm), removing `'a'` from `"a,"` removes the whole text, and the `remove_len >= accumulated_len` branch sets the accumulator back to NULL although `''` is still in the frame (`count` is 1). The next xStep (`update_agg_payload`) sees a NULL accumulator, takes `'éa'` as the first term and adds no separator, so the text is `éa` where it should be `,éa`. Removing `''` then removes `0 + 1` bytes (the first separator length), which is the first byte of `é`. The `is_char_boundary` check turns that into the internal error. With ASCII text the same steps give a wrong result instead of an error, the same one SQLite 3.50.4 gives: with `(1,'a'),(2,''),(3,'bc'),(4,'d')`, Turso returns `'a,'`, `'bc'`, `'c,d'`, `''`, and SQLite trunk returns `'a,'`, `',bc'`, `'bc,d'`, `'d'`.

SQLite's fix resets the accumulator only when no value is left in the frame (`nAccum==0` in place of `str.nChar==0`). The Turso equivalent would be to keep an empty Text accumulator while `count > 0`, and set it to NULL only when `count` reaches 0.

Follow-up to #10352. Tested on Turso main 222c39296 against SQLite 3.50.4 and trunk 8de43901c7.
