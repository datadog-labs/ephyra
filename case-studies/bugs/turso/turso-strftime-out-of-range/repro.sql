SELECT quote(strftime('%Y-%m-%d', 0, '-1 day'));
SELECT quote(strftime('%Y', 1, 'start of year'));
SELECT quote(strftime('%d', 5373484, '+1 day'));
SELECT quote(strftime('x', 0, '-1 day'));
SELECT quote(date(0, '-1 day')), quote(date(5373484, '+1 day'));
