SELECT date('2024-01-01' || char(0) || 'x');
SELECT time(x'3700');
SELECT julianday('2024-01-01' || char(0));
SELECT date('2024-01-01', '+1 day' || char(0) || 'x');
SELECT strftime('%Y' || char(0) || '%m', '2024-05-01');
SELECT length('a' || char(0) || 'b');
