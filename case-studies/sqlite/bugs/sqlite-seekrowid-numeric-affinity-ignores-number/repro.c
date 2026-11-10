#include <stdio.h>
#include "sqlite3.h"

/* f(X): reads X as text, then returns X itself (a copy of the argument). */
static void f(sqlite3_context *ctx, int argc, sqlite3_value **argv){
  (void)argc;
  (void)sqlite3_value_text(argv[0]);
  sqlite3_result_value(ctx, argv[0]);
}

static int cb(void *p, int n, char **v, char **c){
  (void)p; (void)c; (void)n;
  printf("  row: %s\n", v[0] ? v[0] : "NULL");
  return 0;
}

int main(void){
  sqlite3 *db;
  char *zErr = 0;
  printf("%s\n", sqlite3_sourceid());
  sqlite3_open(":memory:", &db);
  sqlite3_create_function(db, "f", 1, SQLITE_UTF8, 0, f, 0, 0);
  sqlite3_exec(db, "CREATE TABLE t(a); INSERT INTO t(rowid, a) VALUES(1, 'one');", 0, 0, 0);
  printf("SELECT a FROM t WHERE rowid = 1.0\n");
  sqlite3_exec(db, "SELECT a FROM t WHERE rowid = 1.0", cb, 0, 0);
  printf("SELECT a FROM t WHERE rowid = f(1.0)\n");
  if( sqlite3_exec(db, "SELECT a FROM t WHERE rowid = f(1.0)", cb, 0, &zErr) ) printf("error: %s\n", zErr);
  sqlite3_close(db);
  return 0;
}
