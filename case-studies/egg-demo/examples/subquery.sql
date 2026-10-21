CREATE TABLE users (id INTEGER PRIMARY KEY, org_id INTEGER NOT NULL, tier INTEGER, name TEXT);
CREATE TABLE orgs (id INTEGER PRIMARY KEY, plan TEXT NOT NULL);
SELECT u.name, u.tier FROM users u
WHERE u.org_id IN (SELECT o.id FROM orgs o WHERE o.plan = 'pro') AND u.tier > 1;
