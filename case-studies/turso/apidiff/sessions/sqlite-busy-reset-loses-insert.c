// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

/* After an INSERT ... SELECT gets SQLITE_BUSY in autocommit mode (left
   unreset), another INSERT on the same connection returns SQLITE_DONE;
   resetting the first statement then discards that INSERT. */
#include <stdio.h>
#include <stdlib.h>
#include "sqlite3.h"
static void ex(sqlite3 *db, const char *sql) {
  char *e = 0; int rc = sqlite3_exec(db, sql, 0, 0, &e);
  printf("  %-45s rc=%d %s\n", sql, rc, e ? e : "");
  sqlite3_free(e);
}
static void count(sqlite3 *db, const char *sql) {
  sqlite3_stmt *s; sqlite3_prepare_v2(db, sql, -1, &s, 0);
  sqlite3_step(s); printf("  %-45s -> %d\n", sql, sqlite3_column_int(s, 0)); sqlite3_finalize(s);
}
int main(int argc, char **argv) {
  const char *path = argv[1];
  remove(path);
  printf("SQLite %s\n", sqlite3_sourceid());
  sqlite3 *a, *b;
  sqlite3_open(path, &a); sqlite3_open(path, &b);
  ex(a, "PRAGMA journal_mode=WAL");
  ex(a, "CREATE TABLE t0(a); CREATE TABLE t2(a)");
  sqlite3_stmt *s1;
  sqlite3_prepare_v2(a, "INSERT INTO t0(a) SELECT a FROM t2", -1, &s1, 0);
  ex(b, "BEGIN EXCLUSIVE");
  printf("  step s1 (INSERT ... SELECT) rc=%d (%s)\n", sqlite3_step(s1), sqlite3_errmsg(a));
  ex(b, "COMMIT");
  ex(a, "INSERT INTO t2 VALUES (1)");
  count(a, "SELECT count(*) FROM t2");
  printf("  reset s1 rc=%d\n", sqlite3_reset(s1));
  count(a, "SELECT count(*) FROM t2");
  count(b, "SELECT count(*) FROM t2");
  sqlite3_finalize(s1);
  sqlite3_close(a); sqlite3_close(b);
  return 0;
}
