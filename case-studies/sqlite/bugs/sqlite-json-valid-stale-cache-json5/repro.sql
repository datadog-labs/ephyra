-- json_valid(X) (flags 1: strict RFC-8259) returns 0 for canonical JSON text
-- produced by json_set()/json_insert()/json_array_insert() from JSON5 input,
-- because the per-statement JSON parse cache reuses the JSON5 input's parse.
-- Expected: every json_valid() below returns 1.

SELECT json_set('[0x1F]', '$[1]', 2);                          -- [31,2]
SELECT json_valid(json_set('[0x1F]', '$[1]', 2));              -- 0, expected 1
SELECT json_set('[0x1F]', '$[1]', 2), json_valid('[31,2]');    -- [31,2]|0, expected [31,2]|1
SELECT json_valid('[31,2]');                                   -- 1 (no earlier call in the statement)

-- Long output (> 100 bytes) from JSON5 input with no hex at all:
SELECT json_valid(json_set('{k:1}', '$.v', printf('%.*c', 120, 'x')));    -- 0, expected 1
SELECT json_valid(json_set('{"k":1}', '$.v', printf('%.*c', 120, 'x')));  -- 1

-- Row-dependent form found by the fuzzer (MATERIALIZED CTE; json_array_insert
-- needs 3.55.0 dev). Expected: both rows X'7C17614B3C176200'
-- ({"a":[{"b":null}]}); trunk gives NULL, NULL. In the fuzzer's query, other
-- JSON calls in the select list pushed the stale entry out of the 4-entry
-- cache after row 1, so row 2 came out right: NULL, X'7C...'.
CREATE TABLE t0(c0); INSERT INTO t0 VALUES(NULL),(NULL);
WITH x0 AS MATERIALIZED (SELECT json_array_insert('[0x1F,-0xa,+1,.5,5.,]', NULL, NULL) AS c0 FROM t0)
SELECT quote(jsonb_patch(CASE WHEN json_valid(c0) THEN c0 END,
                         json_patch('{}', '{a:[{b:null,},],}')))
FROM x0 ORDER BY 1;
