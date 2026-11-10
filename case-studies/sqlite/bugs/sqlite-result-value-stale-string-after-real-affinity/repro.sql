CREATE TABLE r(x REAL);
INSERT INTO r VALUES(sqlar_uncompress(5, 1)) RETURNING x || '';
