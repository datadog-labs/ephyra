SELECT json_valid(X'47E221502E', 8), json_valid(X'29999A', 8);
SELECT json_type(X'47E221502E');
SELECT X'47E221502E' -> '$' IS NOT NULL;
SELECT json_extract(X'29999A', '$') IS NOT NULL;
SELECT json_array_length(X'29999A');
SELECT count(*) FROM json_each(X'47E221502E');
