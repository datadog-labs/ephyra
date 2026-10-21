CREATE TABLE r(x REAL);
CREATE TABLE n(x);
INSERT INTO r VALUES (-0.0);
INSERT INTO n VALUES (-0.0);
SELECT 'real column', atan2(0, x) FROM r;
SELECT 'no affinity', atan2(0, x) FROM n;
