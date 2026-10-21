CREATE TABLE users (id INTEGER PRIMARY KEY, org_id INTEGER NOT NULL, tier INTEGER, name TEXT);
CREATE TABLE orgs (id INTEGER PRIMARY KEY, plan TEXT NOT NULL);
SELECT DISTINCT u.id, o.id, o.plan FROM users u JOIN orgs o ON u.org_id = o.id WHERE o.plan <> 'free';
