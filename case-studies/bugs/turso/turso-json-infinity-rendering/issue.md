DRAFT, not filed

Title: json() rewrites every "9.0e+999" in its output, even inside strings; json_set/json_insert write an infinite REAL as 9.0e+999 where SQLite writes 9e999

```sql
SELECT json('["9.0e+999"]');
SELECT json('{"note":"x 9.0e+999 y"}');
SELECT json('[9.0e+999]');
SELECT json(jsonb_array(9e999));
SELECT json_set('[1]', '$[0]', 9e999), json_insert('[]', '$[#]', -9e999);
SELECT json_set('{}', '$.a', 1e400) = '{"a":9e999}';
```

Turso (`tursodb -q -m list`):

```
["9e999"]
{"note":"x 9e999 y"}
[9e999]
[9e999]
[9.0e+999]|[-9.0e+999]
0
```

SQLite 3.50.4 (trunk is the same):

```
["9.0e+999"]
{"note":"x 9.0e+999 y"}
[9.0e+999]
[9.0e+999]
[9e999]|[-9e999]
1
```

Expected: the SQLite output. The first two queries change the contents of a JSON string. In SQLite, json() keeps a number's text as written (and as stored in JSONB), so `9.0e+999` stays `9.0e+999`. Only a REAL infinity passed straight to json() (`json(9e999)`) comes out as `9e999`, and Turso already matches that. In the other direction, SQLite's json_set/json_insert/json_replace write an infinite REAL value as `9e999`, while Turso writes `9.0e+999` (the spelling json_array/json_object/json_quote use).

Likely cause: `get_json` (core/json/mod.rs:56-59) runs `json.replace("9.0e+999", "9e999")` on the whole serialized document, string contents included. That was added for #4196 to make `json(<infinite REAL>)` print `9e999`. The REAL-to-JSONB conversion (core/json/mod.rs ~279) always writes `9.0e+999`. SQLite writes `9e999` for json_set/json_insert values and in json() of a REAL, and `9.0e+999` in json_array/json_object/json_quote.

Tested on Turso main f8549ca2.
