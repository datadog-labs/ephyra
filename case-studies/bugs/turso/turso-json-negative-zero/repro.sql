SELECT json_array(-0.0), json_object('a', -0.0), json_quote(-0.0), json(jsonb_array(-0.0));
SELECT jsonb_array(-0.0) = jsonb_array(0.0);
SELECT count(*) FROM (SELECT DISTINCT json_array(x) FROM (SELECT -0.0 AS x UNION ALL SELECT 0.0));
-- outside JSON both engines print 0.0:
SELECT -0.0, CAST(-0.0 AS TEXT), quote(-0.0);
-- JSON text is passed through unchanged by both:
SELECT json('[-0.0]');
