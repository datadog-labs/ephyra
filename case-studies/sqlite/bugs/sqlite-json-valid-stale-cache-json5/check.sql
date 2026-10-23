-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-json-valid-stale-cache-json5
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
SELECT json_valid(json_set('[0x1F]', '$[1]', 2));
SELECT 'q2';
SELECT json_set('[0x1F]', '$[1]', 2), json_valid('[31,2]');
SELECT 'q3';
SELECT json_valid(json_set('{k:1}', '$.v', printf('%.*c', 120, 'x')));
CREATE TABLE t0(c0);
INSERT INTO t0 VALUES(NULL),(NULL);
SELECT 'q4';
WITH x0 AS MATERIALIZED (SELECT json_array_insert('[0x1F,-0xa,+1,.5,5.,]', NULL, NULL) AS c0 FROM t0)
SELECT quote(jsonb_patch(CASE WHEN json_valid(c0) THEN c0 END,
                         json_patch('{}', '{a:[{b:null,},],}')))
FROM x0 ORDER BY 1;
