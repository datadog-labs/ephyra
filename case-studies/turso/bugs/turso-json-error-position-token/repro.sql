SELECT json_error_position('1x');
SELECT json_error_position('"a"x');
SELECT json_error_position('1 x');
SELECT json_error_position('2023-01-');
SELECT json_error_position('[truex]');
SELECT json_error_position('1e');
SELECT json_error_position('[1e]');
SELECT json_error_position('{"a":1e}');
SELECT json_error_position('"\x"');
