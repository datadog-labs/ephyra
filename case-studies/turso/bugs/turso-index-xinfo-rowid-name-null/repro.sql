CREATE TABLE t(a);
CREATE INDEX i ON t(a);
SELECT seqno, cid, quote(name), key FROM pragma_index_xinfo('i');
