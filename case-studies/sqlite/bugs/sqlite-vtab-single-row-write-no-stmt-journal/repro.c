#include <stdio.h>
#include "sqlite3.h"

/* An FTS5 tokenizer that splits on spaces and rejects non-ASCII text. */
static int xCreate(void *p, const char **az, int n, Fts5Tokenizer **pp){
  *pp = (Fts5Tokenizer*)p;
  return SQLITE_OK;
}
static void xDelete(Fts5Tokenizer *p){}
static int xTokenize(Fts5Tokenizer *p, void *pCtx, int flags,
                     const char *z, int n,
                     int (*xToken)(void*, int, const char*, int, int, int)){
  int i = 0, j, rc = SQLITE_OK;
  while( rc==SQLITE_OK && i<n ){
    while( i<n && z[i]==' ' ) i++;
    for(j=i; j<n && z[j]!=' '; j++){
      if( (unsigned char)z[j]>=0x80 ) return SQLITE_ERROR;
    }
    if( j>i ) rc = xToken(pCtx, 0, z+i, j-i, i, j);
    i = j;
  }
  return rc;
}

static int print_row(void *p, int n, char **az, char **azCol){
  int i;
  for(i=0; i<n; i++) printf("%s%s", i ? "|" : "", az[i]);
  printf("\n");
  return 0;
}
static void run(sqlite3 *db, const char *zSql){
  char *zErr = 0;
  if( sqlite3_exec(db, zSql, print_row, 0, &zErr) ){
    printf("%s: %s\n", zSql, zErr);
    sqlite3_free(zErr);
  }
}

int main(void){
  sqlite3 *db;
  sqlite3_stmt *pStmt;
  fts5_api *pApi = 0;
  fts5_tokenizer tok = { xCreate, xDelete, xTokenize };

  sqlite3_open(":memory:", &db);
  sqlite3_prepare_v2(db, "SELECT fts5(?1)", -1, &pStmt, 0);
  sqlite3_bind_pointer(pStmt, 1, (void*)&pApi, "fts5_api_ptr", 0);
  sqlite3_step(pStmt);
  sqlite3_finalize(pStmt);
  pApi->xCreateTokenizer(pApi, "ascii", 0, &tok, 0);

  run(db, "CREATE VIRTUAL TABLE ft USING fts5(x, tokenize=ascii)");
  run(db, "INSERT INTO ft VALUES('one')");
  run(db, "BEGIN");
  run(db, "INSERT INTO ft VALUES('two caf\xc3\xa9')");
  run(db, "COMMIT");
  run(db, "SELECT rowid, x FROM ft");
  run(db, "SELECT rowid FROM ft('two')");
  run(db, "INSERT INTO ft(ft) VALUES('integrity-check')");
  sqlite3_close(db);
  return 0;
}
