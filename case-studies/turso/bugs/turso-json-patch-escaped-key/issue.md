Filed 2026-10-21 as https://github.com/tursodatabase/turso/issues/10370 (the filed text adds the PR #9913 note and the fe4beb8 version line).
Title: json_patch/jsonb_patch decode the escapes in the keys they add, and jsonb_patch writes a key with a raw control character as plain TEXT

When json_patch adds a member from the patch to the target object, Turso unescapes the key before inserting it. json_patch then prints `é` where SQLite keeps `\u00e9`. jsonb_patch stores the decoded key as a TEXT element, the JSONB type for text that needs no escaping. If the key contained `\n`, the blob now has a raw newline inside a TEXT key. That blob fails `json_valid(x, 8)` in both engines, and SQLite renders it as `{"x<newline>y":1}`, which isn't valid JSON. Values are not affected, only keys.

```sql
SELECT 1, json_patch('{}', '{"\u00e9":1}');
SELECT 2, json_patch('{"a":1}', '{"b\u00e9":2}');
SELECT 3, json_patch('{"a":1}', '{"a":"\u00e9"}');
SELECT 4, hex(jsonb_patch('{}', '{"x\ny":1}'));
SELECT 5, json_valid(jsonb_patch('{}', '{"x\ny":1}'), 8);
```

Turso:

```
1|{"é":1}
2|{"a":1,"bé":2}
3|{"a":"\u00e9"}
4|6C37780A791331
5|0
```

SQLite 3.50.4 (trunk gives the same):

```
1|{"\u00e9":1}
2|{"a":1,"b\u00e9":2}
3|{"a":"\u00e9"}
4|7C48785C6E791331
5|1
```

Expected: SQLite's output. Like SQLite's other JSON functions, json_patch copies a key's text as it was written (`7C 48 78 5C 6E 79`: a TEXTJ key `x\ny`). Turso's blob has `37 78 0A 79`, a 3-byte TEXT key containing a raw newline. The JSONB spec does not allow that for the TEXT type.

Likely cause: the patch loop in core/json/jsonb.rs (`Jsonb::patch`, ~3300) reads each patch key, calls `unescape_string` on TEXT5/TEXTJ/TEXTRAW keys, and inserts the member through a `PathElement::Key` path. The inserted key is written from the decoded string, so its original escapes and element type are lost.

Tested on Turso main abf885b2.
