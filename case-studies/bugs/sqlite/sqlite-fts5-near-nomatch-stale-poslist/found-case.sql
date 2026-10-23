-- Found case (run 20261021-2000, w0-23261). Run on a copy of found-before.db (holds t0..t4):
--   cp found-before.db x.db; sqlite3 x.db < found-case.sql
-- Trunk 3769f60be38b: "database disk image is malformed" at the last statement; with fix.diff: 1

CREATE VIRTUAL TABLE vt0 USING rtree(k, l0, h0, l1, h1);
CREATE VIRTUAL TABLE vt1 USING fts5(s0, s1, tokenize=[porter unicode61 remove_diacritics 2], L='1');
INSERT INTO vt1(rowid, s0, s1) VALUES (42, 'DATE', fts5_locale('en', 'chess zeta, hesitanci eagle. decisiveness, delta'));
INSERT INTO vt1(rowid, s0, s1) VALUES (51, fts5_locale('en', 'operator inference echo running adjustable HESITANCI. DELTA, caresses'), fts5_locale('', 'FEUDALISM-triplicate'));
.dbconfig dqs_dml on
WITH x0 AS MATERIALIZED (SELECT +rowid AS c0, s0 AS c1, s1 AS c2, snippet(vt1, -1, '<', '>', '..', 3) AS c3 FROM vt1 WHERE vt1 MATCH 's1 : NEAR(hesitanci delta, 2) OR ru* OR café + rela*' LIMIT -1), x1 AS NOT MATERIALIZED (SELECT +rowid AS c0, s0 AS c1, s1 AS c2 FROM vt1 WHERE vt1 MATCH '{s0 s1} : u0' AND vt1 MATCH 'zeta OR apple OR t13' AND rank MATCH 'bm25(14.5, 1.0, 1)') SELECT count(*) FROM t2 AS a0 JOIN (SELECT * FROM t4 LIMIT -1) AS a1 ON (NOT (json_quote(jsonb_extract('false', '$')) <= 5.5)) FULL JOIN t3 AS a2 ON ((a2.c0 = a0."c3") AND ((a2.c0, a2.c0) IN (SELECT c0, c0 FROM (SELECT * FROM t4 LIMIT -1) AS b0 WHERE ((c0 <> 001) AND c0)))) FULL OUTER JOIN x0 AS a3 ON ((a3.c0 = a0.c0) AND (CASE WHEN ((round(x'00', (5 || 5.5)) || quote(a2.c0)) OR a2.c0) THEN a3."c3" ELSE ((- ((a0.c1 NOT BETWEEN a2.c0 AND a3.c0) IS NOT DISTINCT FROM (0 != a0.c1))) <> (CASE WHEN json_type("[[{""x y"":'a\ b'}, ""\x41""],{""é"":{}, ""0"":{},'b':[""é"",""中文"",'\uDBFF\uDFFF','中文']},]") THEN a0.c2 ELSE 45.5 END)) END)) WHERE ((a1.c0 NOT IN (SELECT c1 FROM main.t0 AS b0)) AND (a1.c0 = a0."c3"));
