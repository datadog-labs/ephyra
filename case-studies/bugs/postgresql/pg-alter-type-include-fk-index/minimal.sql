CREATE TABLE p(a int4, b int4);
CREATE UNIQUE INDEX p_u ON p (a) INCLUDE (b);
CREATE TABLE c(a int4 REFERENCES p (a));
ALTER TABLE p ALTER COLUMN b TYPE int8;
-- ERROR:  cannot drop index p_u because other objects depend on it  (2BP01)
-- DETAIL:  constraint c_a_fkey on table c depends on index p_u
