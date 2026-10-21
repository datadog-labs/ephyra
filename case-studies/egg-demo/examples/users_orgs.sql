CREATE TABLE users (id INTEGER PRIMARY KEY, org_id INTEGER NOT NULL, tier INTEGER, name TEXT);
CREATE TABLE orgs (id INTEGER PRIMARY KEY, plan TEXT NOT NULL);
SELECT u.name, o.plan
FROM users u JOIN orgs o ON u.org_id = o.id
WHERE u.tier IN (1, 2) AND NOT (o.plan = 'free') AND u.org_id IS NOT NULL;
