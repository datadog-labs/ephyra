CREATE TABLE w(x, PRIMARY KEY(x)) WITHOUT ROWID;
SELECT 'list', * FROM pragma_index_list('w');
SELECT 'info', * FROM pragma_index_info('sqlite_autoindex_w_1');
SELECT 'xinfo', * FROM pragma_index_xinfo('sqlite_autoindex_w_1');
