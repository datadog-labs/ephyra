\set VERBOSITY verbose
CREATE EXTENSION btree_gist;
CREATE TABLE t (d date);
CREATE INDEX t_d_idx ON t USING gist (d);
-- about 410+ rows are enough for the index to grow past one page
INSERT INTO t SELECT date '2000-01-01' + i FROM generate_series(1, 500) i;
INSERT INTO t VALUES ('infinity');
INSERT INTO t VALUES ('-infinity');
-- an index built over rows that already hold infinities (sorted build)
-- succeeds, but later inserts and KNN searches fail
CREATE TABLE u (d date);
INSERT INTO u SELECT date '2000-01-01' + i FROM generate_series(1, 500) i;
INSERT INTO u VALUES ('infinity'), ('-infinity');
CREATE INDEX u_d_idx ON u USING gist (d);
INSERT INTO u VALUES ('2000-06-01');
SET enable_seqscan = off;
SELECT d FROM u ORDER BY d <-> date '2000-06-01' LIMIT 1;
-- the scalar operator rejects infinity as well
SELECT date 'infinity' <-> date '2000-01-01';
-- for comparison, timestamp handles all of this
CREATE TABLE ts (x timestamp);
CREATE INDEX ts_x_idx ON ts USING gist (x);
INSERT INTO ts SELECT timestamp '2000-01-01' + i * interval '1 day' FROM generate_series(1, 500) i;
INSERT INTO ts VALUES ('infinity'), ('-infinity');
SELECT timestamp 'infinity' <-> timestamp '2000-01-01';
