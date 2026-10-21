SELECT quote(json_set('null', '$.a', 1));
SELECT quote(json_replace('{}', '$', NULL));
SELECT quote(jsonb_set('[1]', '$', NULL));
SELECT quote(json_set(NULL, '$', 5));
SELECT quote(json_set('[1]', NULL, x'ff'));
SELECT quote(json_remove('[1]', NULL));
