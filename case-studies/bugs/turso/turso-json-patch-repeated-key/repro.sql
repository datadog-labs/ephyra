SELECT json(jsonb_patch('{}', '{"e":{"f":{"a":1}},"e":null}'));
SELECT json_patch('{}', '{"e":{"f":1},"e":null}');
SELECT json_patch('{"e":[1]}', '{"e":{"x":1},"e":null}');
SELECT json(jsonb_patch('{"e":[1]}', '{"e":{"0":{"a":1},"0":{}}}'));
