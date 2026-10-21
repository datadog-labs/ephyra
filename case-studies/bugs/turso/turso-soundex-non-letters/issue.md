Filed as #10356 (https://github.com/tursodatabase/turso/issues/10356) on 2026-10-21.
Title: soundex() returns '?000' for any input with a non-letter, and codes H/W differently from SQLite

```sql
SELECT soundex('Ashcraft'), soundex(' Robert'), soundex('O''Brien'), soundex('Ab-bc'), soundex('1abc'), soundex(x'41'), soundex(4.5e-10);
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
A261|?000|?000|?000|?000|?000|?000
```

SQLite 3.50.4 built with `-DSQLITE_SOUNDEX` (soundex() only exists in such builds):
```
A226|R163|O165|A112|A120|A000|E000
```

Expected: the SQLite output. SQLite's `soundexFunc` (func.c):
- converts the argument to text, so a BLOB or a number is coded from its text (`4.5e-10` has the letter `e`);
- skips leading non-letters, and returns `?000` only when the text has no letter at all;
- treats every character without a code (vowels, `h`, `w`, `y`, spaces, digits, punctuation) as a separator that resets the previous code, so `Ashcraft` is `A226`.

Turso returns `?000` for anything that is not TEXT, and for any text that contains a byte that is not an ASCII letter. It also skips `h` and `w` without resetting the previous code (the American Soundex rule), giving `A261` for `Ashcraft`.

Likely cause: `Value::exec_soundex` (core/vdbe/value.rs): the `s.bytes().any(|b| !b.is_ascii_alphabetic())` early return, the `_ => "?000"` arm for non-text values, and the `continue` for `h`/`w`.

Tested on Turso main cbd5ac4e.
