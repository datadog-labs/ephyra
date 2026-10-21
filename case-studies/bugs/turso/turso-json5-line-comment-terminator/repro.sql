SELECT json('[1,// c2]');
SELECT json('[1,// c'||char(0x2028)||'2]');
SELECT json('[1,// c'||char(0x2029)||'2]');
SELECT json('[1,// c'||char(10)||'2]');
