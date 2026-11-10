/* Two UPDATE ... RETURNING statements, each stepped once and left
** pending, then the older one finished first.
** ./repro      the older one runs to completion
** ./repro oom  the older one fails with SQLITE_NOMEM on its next step */
#include <stdio.h>
#include <string.h>
#include "sqlite3.h"

static sqlite3 *db;

static void finish(const char *zName, sqlite3_stmt *p){
  int rc;
  while( (rc = sqlite3_step(p))==SQLITE_ROW ){}
  printf("%s: %s\n", zName, sqlite3_errstr(rc));
}

int main(int argc, char **argv){
  int oom = argc>1 && strcmp(argv[1], "oom")==0;
  sqlite3_stmt *pA, *pB, *pS;
  sqlite3_open(":memory:", &db);
  sqlite3_exec(db, "CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE);"
                   "INSERT INTO t VALUES(1,1),(2,2),(3,3);"
                   "BEGIN;", 0, 0, 0);
  sqlite3_prepare_v2(db,
      "UPDATE t SET b=b+10 RETURNING a, randomblob(100000*a)", -1, &pA, 0);
  sqlite3_prepare_v2(db, "UPDATE t SET b=b+100 RETURNING a", -1, &pB, 0);
  sqlite3_step(pA);
  sqlite3_step(pB);
  if( oom ) sqlite3_hard_heap_limit64(sqlite3_memory_used() + 1000);
  finish("A", pA);
  sqlite3_hard_heap_limit64(0);
  finish("B", pB);
  sqlite3_finalize(pA);
  sqlite3_finalize(pB);
  printf("COMMIT: %s\n", sqlite3_errstr(sqlite3_exec(db, "COMMIT", 0, 0, 0)));
  sqlite3_prepare_v2(db, "SELECT a, b FROM t", -1, &pS, 0);
  while( sqlite3_step(pS)==SQLITE_ROW ){
    printf("%d|%d\n", sqlite3_column_int(pS, 0), sqlite3_column_int(pS, 1));
  }
  sqlite3_finalize(pS);
  sqlite3_close(db);
  return 0;
}
