CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT, score INTEGER NOT NULL, owner INTEGER);
SELECT e.id, e.kind FROM events e
WHERE e.score BETWEEN 10 AND 20 AND NOT (e.kind = 'test' OR e.owner IS NULL);
