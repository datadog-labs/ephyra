-- A self-referencing foreign key with ON UPDATE CASCADE on a partitioned
-- table: moving a row that references itself to another partition fails,
-- while the same UPDATE on a plain table cascades.
CREATE TABLE sp (c0 int8 PRIMARY KEY, c1 int8) PARTITION BY RANGE (c0);
CREATE TABLE sp_0 PARTITION OF sp FOR VALUES FROM (936) TO (MAXVALUE);
CREATE TABLE sp_1 PARTITION OF sp DEFAULT;
ALTER TABLE sp ADD FOREIGN KEY (c1) REFERENCES sp (c0) ON UPDATE CASCADE;
INSERT INTO sp VALUES (104, 104);
UPDATE sp SET c0 = 936;  -- ERROR: insert or update on table "sp_0" violates foreign key constraint
TABLE sp;

CREATE TABLE sn (c0 int8 PRIMARY KEY, c1 int8 REFERENCES sn (c0) ON UPDATE CASCADE);
INSERT INTO sn VALUES (104, 104);
UPDATE sn SET c0 = 936;  -- UPDATE 1, row (936, 936)
TABLE sn;
