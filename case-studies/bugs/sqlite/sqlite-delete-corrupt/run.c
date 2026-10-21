#include <stdio.h>
#include <stdlib.h>
#include "sqlite3.h"
static int cb(void *u, int n, char **v, char **c) { for (int i = 0; i < n; i++) printf("%s\n", v[i] ? v[i] : "NULL"); return 0; }
int main(int argc, char **argv) {
  FILE *f = fopen(argv[1], "rb"); fseek(f, 0, SEEK_END); long n = ftell(f); rewind(f);
  char *sql = malloc(n + 1); fread(sql, 1, n, f); sql[n] = 0;
  sqlite3 *db; sqlite3_open(":memory:", &db);
  printf("sqlite %s\n", sqlite3_libversion());
  char *err = 0; const char *tail = sql; sqlite3_stmt *st;
  /* run statement by statement so an error doesn't stop the script */
  while (*tail) {
    if (sqlite3_prepare_v2(db, tail, -1, &st, &tail) != SQLITE_OK) { printf("prepare error: %s\n", sqlite3_errmsg(db)); break; }
    if (!st) continue;
    int rc;
    while ((rc = sqlite3_step(st)) == SQLITE_ROW) printf("%s\n", sqlite3_column_text(st, 0));
    if (rc != SQLITE_DONE) printf("error: %s (%d) in: %s\n", sqlite3_errmsg(db), rc, sqlite3_sql(st));
    sqlite3_finalize(st);
  }
  return 0;
}
