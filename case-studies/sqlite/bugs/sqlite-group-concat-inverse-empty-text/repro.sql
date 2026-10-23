-- group_concat over a moving frame. The inverse step that removes 'a' leaves
-- the text empty while one term ('') is still in the frame; the next step then
-- treats its term as the first and writes no separator, and the inverse that
-- later removes '' cuts the separator's length off the following text.
CREATE TABLE t1(a INTEGER PRIMARY KEY, b);
INSERT INTO t1 VALUES(1, ''), (2, ''), (3, '');
SELECT quote(group_concat(b) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t1;
-- ','  ''  ''     expected ','  ','  ''

CREATE TABLE t2(a INTEGER PRIMARY KEY, b);
INSERT INTO t2 VALUES(1, 'a'), (2, ''), (3, 'b');
SELECT quote(group_concat(b, '-') OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t2;
-- 'a-'  'b'  ''   expected 'a-'  '-b'  'b'
