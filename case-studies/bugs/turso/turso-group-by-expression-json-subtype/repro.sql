CREATE TABLE t(a, b);
INSERT INTO t VALUES (1, 'x'), (2, 'y');
SELECT json_array(json_object('k', b)) FROM t GROUP BY json_object('k', b);
SELECT json_quote(json_quote(b)) FROM t GROUP BY json_quote(b);
SELECT json_array(json(a || '')) FROM t GROUP BY json(a || '');
SELECT json_array(json_object('k', b)) FROM t GROUP BY 1;
