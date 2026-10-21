CREATE TABLE users (id INTEGER PRIMARY KEY, org_id INTEGER NOT NULL, tier INTEGER);
CREATE TABLE orgs (id INTEGER PRIMARY KEY, region TEXT);
CREATE TABLE invoices (id INTEGER PRIMARY KEY, org_id INTEGER NOT NULL, amount INTEGER);
SELECT u.id, i.amount FROM users u JOIN orgs o ON u.org_id = o.id JOIN invoices i ON i.org_id = o.id
WHERE i.amount > 100 AND u.tier IN (1, 2, 3) AND o.region <> 'eu';
