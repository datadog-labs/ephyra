SELECT json_insert('[]', '$', 1);
SELECT json_insert('{"a":2}', '$', 1);
SELECT json_insert('[5]', '$', json('{"b":1}'));
SELECT json(jsonb_insert('[5]', '$', 1));
SELECT json_insert('{"a":2}', '$.a', 1);
