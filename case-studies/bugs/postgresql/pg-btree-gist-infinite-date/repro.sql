-- btree_gist: inserting an infinite date into a GiST index of more than one page fails.
-- PostgreSQL master 158b6d5109 (2026-10-13), cassert build.
CREATE EXTENSION btree_gist;
CREATE TABLE t(d date);
CREATE INDEX ON t USING gist (d);
INSERT INTO t SELECT '2024-01-01'::date + (i % 400) FROM generate_series(1, 2000) i;
INSERT INTO t VALUES ('-infinity');   -- ERROR:  cannot subtract infinite dates
INSERT INTO t VALUES ('infinity');    -- ERROR:  cannot subtract infinite dates
-- The same values load when the index is built afterwards:
CREATE TABLE u(d date);
INSERT INTO u SELECT '2024-01-01'::date + (i % 400) FROM generate_series(1, 2000) i;
INSERT INTO u VALUES ('-infinity'), ('infinity');
CREATE INDEX ON u USING gist (d);
