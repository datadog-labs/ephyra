SELECT json_set('[1,2]', '$[#]', NULL);
SELECT json_set('[1,2]', '$[#]', 1234567, '$[#]', 5);
SELECT json_insert('[]', '$[0]', json('[]'));
SELECT json_set('[1,2]', '$[#]', 'ab', '$[#]', 'cd');
SELECT hex(jsonb_set('[1,2]', '$[#]', 3));
