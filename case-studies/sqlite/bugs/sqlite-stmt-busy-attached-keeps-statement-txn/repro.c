#include <stdio.h>
#include "sqlite3.h"

static void finish(const char *zName, sqlite3_stmt *p){
  while( sqlite3_step(p)==SQLITE_ROW ){}
  printf("%s: %s\n", zName, sqlite3_errstr(sqlite3_finalize(p)));
}

int main(void){
  sqlite3 *db, *db2;
  sqlite3_stmt *pP, *pA, *pB, *pS;
  remove("m.db"); remove("x.db");
  sqlite3_open("m.db", &db);
  sqlite3_exec(db, "ATTACH 'x.db' AS x;"
                   "CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE);"
                   "INSERT INTO t VALUES(1,1),(2,2),(3,3);"
                   "CREATE TABLE x.u(v);"
                   "CREATE TEMP TRIGGER tr AFTER UPDATE ON t BEGIN"
                   "  INSERT INTO u VALUES(new.b);"
                   "END;", 0, 0, 0);
  sqlite3_open("x.db", &db2);
  sqlite3_exec(db2, "BEGIN IMMEDIATE", 0, 0, 0);   /* x.db is locked */

  /* Autocommit mode.  Each UPDATE also writes x.db through the trigger. */
  sqlite3_prepare_v2(db, "UPDATE t SET b=b WHERE a=99", -1, &pP, 0);
  sqlite3_prepare_v2(db, "UPDATE t SET b=b+10", -1, &pA, 0);
  sqlite3_prepare_v2(db,
      "UPDATE t SET b=CASE a WHEN 2 THEN 13 ELSE b+100 END", -1, &pB, 0);
  printf("P: %s\n", sqlite3_errstr(sqlite3_step(pP)));
  printf("A: %s\n", sqlite3_errstr(sqlite3_step(pA)));
  printf("B: %s\n", sqlite3_errstr(sqlite3_step(pB)));
  sqlite3_exec(db2, "COMMIT", 0, 0, 0);            /* x.db is unlocked */
  finish("A", pA);                                 /* retry, oldest first */
  finish("B", pB);
  finish("P", pP);

  sqlite3_prepare_v2(db, "SELECT (SELECT group_concat(a||'|'||b, ' ') FROM t),"
                         " (SELECT group_concat(v) FROM x.u)", -1, &pS, 0);
  sqlite3_step(pS);
  printf("t: %s\nx.u: %s\n", sqlite3_column_text(pS, 0),
         sqlite3_column_text(pS, 1));
  sqlite3_finalize(pS);
  sqlite3_close(db2);
  sqlite3_close(db);
  return 0;
}
