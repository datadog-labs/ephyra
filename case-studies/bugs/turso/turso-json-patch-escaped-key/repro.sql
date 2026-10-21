SELECT 1, json_patch('{}', '{"\u00e9":1}');
SELECT 2, json_patch('{"a":1}', '{"b\u00e9":2}');
SELECT 3, json_patch('{"a":1}', '{"a":"\u00e9"}');
SELECT 4, hex(jsonb_patch('{}', '{"x\ny":1}'));
SELECT 5, json_valid(jsonb_patch('{}', '{"x\ny":1}'), 8);
