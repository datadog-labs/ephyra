Filed as #9885 (https://github.com/tursodatabase/turso/issues/9885) (line 1 corrected 2026-10-21).

# json_pretty() prints an empty object or array on two lines

## Reproduction

```sql
SELECT json_pretty('{}');
SELECT json_pretty('[]');
SELECT json_pretty('{"a":[],"b":{}}');
```

Turso (`tursodb -q -m list`, main f8549ca2):
```
{
}
[
]
{
    "a": [
    ],
    "b": {
    }
}
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same):
```
{}
[]
{
    "a": [],
    "b": {}
}
```

## Expected

`{}` and `[]`, as SQLite prints them. Any JSON with an empty container pretty-prints differently, so comparisons, DISTINCT and GROUP BY over `json_pretty()` output differ too.

## Likely cause

`serialize_object` and `serialize_array` in `core/json/jsonb.rs` push `'\n'` right after `{`/`[` whenever indentation is on, before checking whether the container has any element.

Tested on Turso main f8549ca2.
