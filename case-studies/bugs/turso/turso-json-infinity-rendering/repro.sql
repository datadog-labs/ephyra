SELECT json('["9.0e+999"]');
SELECT json('{"note":"x 9.0e+999 y"}');
SELECT json('[9.0e+999]');
SELECT json(jsonb_array(9e999));
SELECT json_set('[1]', '$[0]', 9e999), json_insert('[]', '$[#]', -9e999);
SELECT json_set('{}', '$.a', 1e400) = '{"a":9e999}';
