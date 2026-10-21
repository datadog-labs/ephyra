DRAFT, not filed. Duplicate of open issue #8538 ("jsonb_insert and jsonb_set that grow an array return a malformed blob"). If posted, post as a comment on #8538: it shows that the text functions lose data too.

Comment text:

The same header bug also loses data in the text-returning json_set/json_insert. #8538 says "json_insert('[]','$[#]',1) is fine", but that only holds when the appended element has a non-empty payload. If the new element is a header-only value (null, true, false, '', [], {}), it is dropped. And when a second edit follows, the short header truncates the earlier value:

```sql
SELECT json_set('[1,2]', '$[#]', NULL);
SELECT json_set('[1,2]', '$[#]', json('true'));
SELECT json_insert('[]', '$[0]', json('[]'));
SELECT json_set('[1,2]', '$[#]', 1234567, '$[#]', 5);
SELECT json_set('[1,2]', '$[#]', 'ab', '$[#]', 'cd');
SELECT json_set('[]', '$[0]', 1, '$[1]', 2);
```

| query | SQLite 3.50.4 (and trunk) | Turso 7641fad2 |
|---|---|---|
| append NULL | `[1,2,null]` | `[1,2]` |
| append true | `[1,2,true]` | `[1,2]` |
| insert `[]` into `[]` | `[[]]` | `[]` |
| append 1234567 then 5 | `[1,2,1234567,5]` | `[1,2,123456]` |
| append 'ab' then 'cd' | `[1,2,"ab","cd"]` | error: Invalid JSONB: number extends beyond data |
| set $[0]=1 then $[1]=2 on `[]` | `[1,2]` | `[1]` |

Only a top-level array is affected, as #8538 says (`json_set('{"a":[1]}','$.a[#]',1234567,'$.a[#]',5)` and `json_set('[[1]]','$[0][#]',...)` are correct). In `SetOperation::execute` (core/json/jsonb.rs ~366) the root/object-property branch rewrites the array header with `obj_value_size + delta`. `obj_value_size` was read after `navigate_to_segment` spliced in the 1-byte placeholder, but the header doesn't include it yet, and `target.delta` (the placeholder byte) is never added. So the root header is one byte short for each appended element.
