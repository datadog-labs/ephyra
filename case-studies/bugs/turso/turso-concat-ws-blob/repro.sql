SELECT quote(concat_ws(',', 'a', x'62', 'c'));
SELECT quote(concat_ws(x'2c', 'a', 'b'));
SELECT quote(concat_ws(x'', NULL));
