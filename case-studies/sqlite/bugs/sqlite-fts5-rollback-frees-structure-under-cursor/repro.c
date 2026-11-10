#include <stdio.h>
#include "sqlite3.h"
static void ex(sqlite3 *db, const char *sql){
  char *err = 0;
  int rc = sqlite3_exec(db, sql, 0, 0, &err);
  printf("exec %-40s rc=%d %s\n", sql, rc, err ? err : "");
  sqlite3_free(err);
}
int main(void){
  sqlite3 *db; sqlite3_stmt *q; int rc;
  printf("SQLite %s\n", sqlite3_sourceid());
  sqlite3_open(":memory:", &db);
  ex(db, "CREATE VIRTUAL TABLE ft USING fts5(a)");
  ex(db, "INSERT INTO ft VALUES('x'); INSERT INTO ft VALUES('x')");
  ex(db, "BEGIN");
  ex(db, "INSERT INTO ft VALUES('y')");
  sqlite3_prepare_v2(db, "SELECT rowid FROM ft WHERE ft MATCH 'x'", -1, &q, 0);
  rc = sqlite3_step(q);
  printf("step 1 rc=%d rowid=%lld\n", rc, sqlite3_column_int64(q, 0));
  ex(db, "ROLLBACK");
  rc = sqlite3_step(q);
  printf("step 2 rc=%d %s\n", rc, sqlite3_errmsg(db));
  sqlite3_finalize(q);
  sqlite3_close(db);
  return 0;
}
