// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

/*
** The memvt module of ../src/lib.rs for SQLite: columns (a, b), rows in
** memory in one process-wide store shared by every connection, xBegin
** snapshots, xCommit drops the snapshot, xRollback restores it. Hook calls
** are printed to stderr as "[memvt] ...".
**
** Driver: memvt_sqlite DB SCRIPT. Each script line is one SQL statement run
** on connection "a"; a line starting with "@b " or "@c " runs on a second or
** third connection (each registers the module). Results and errors are
** printed like the Turso repro programs print them.
*/
#include "sqlite3.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct { sqlite3_int64 rowid; sqlite3_value *a, *b; } Row;
typedef struct { Row *r; int n; } Rows;
static Rows rows, before; static int have_before = 0;

static void rows_free(Rows *x){ for(int i=0;i<x->n;i++){ sqlite3_value_free(x->r[i].a); sqlite3_value_free(x->r[i].b);} free(x->r); x->r=0; x->n=0; }
static Rows rows_dup(Rows *x){ Rows y; y.n=x->n; y.r=malloc(sizeof(Row)*(x->n+1)); for(int i=0;i<x->n;i++){ y.r[i].rowid=x->r[i].rowid; y.r[i].a=sqlite3_value_dup(x->r[i].a); y.r[i].b=sqlite3_value_dup(x->r[i].b);} return y; }
static int find(sqlite3_int64 id){ for(int i=0;i<rows.n;i++) if(rows.r[i].rowid==id) return i; return -1; }
static void logm(const char *m){ fprintf(stderr, "  [memvt] %s\n", m); }

typedef struct { sqlite3_vtab base; } Tab;
typedef struct { sqlite3_vtab_cursor base; Rows snap; int pos; } Cur;

static int xCreate(sqlite3 *db, void *p, int argc, const char *const *argv, sqlite3_vtab **pp, char **err){
  int rc = sqlite3_declare_vtab(db, "CREATE TABLE x(a, b)"); if(rc) return rc;
  Tab *t = sqlite3_malloc(sizeof(Tab)); memset(t,0,sizeof(*t)); *pp=&t->base; return SQLITE_OK; }
static int xCreateL(sqlite3 *db, void *p, int argc, const char *const *argv, sqlite3_vtab **pp, char **err){ logm("create"); return xCreate(db,p,argc,argv,pp,err); }
static int xBestIndex(sqlite3_vtab *t, sqlite3_index_info *i){ i->estimatedCost=1000; return SQLITE_OK; }
static int xDisconnect(sqlite3_vtab *t){ sqlite3_free(t); return SQLITE_OK; }
static int xDestroy(sqlite3_vtab *t){ logm("destroy"); sqlite3_free(t); return SQLITE_OK; }
static int xOpen(sqlite3_vtab *t, sqlite3_vtab_cursor **pp){ Cur *c=calloc(1,sizeof(Cur)); *pp=&c->base; return SQLITE_OK; }
static int xClose(sqlite3_vtab_cursor *c){ rows_free(&((Cur*)c)->snap); free(c); return SQLITE_OK; }
static int xFilter(sqlite3_vtab_cursor *c, int n, const char *s, int argc, sqlite3_value **argv){ Cur *k=(Cur*)c; rows_free(&k->snap); k->snap=rows_dup(&rows); k->pos=0; return SQLITE_OK; }
static int xNext(sqlite3_vtab_cursor *c){ ((Cur*)c)->pos++; return SQLITE_OK; }
static int xEof(sqlite3_vtab_cursor *c){ Cur *k=(Cur*)c; return k->pos>=k->snap.n; }
static int xColumn(sqlite3_vtab_cursor *c, sqlite3_context *ctx, int i){ Cur *k=(Cur*)c; sqlite3_result_value(ctx, i==0?k->snap.r[k->pos].a:k->snap.r[k->pos].b); return SQLITE_OK; }
static int xRowid(sqlite3_vtab_cursor *c, sqlite3_int64 *r){ Cur *k=(Cur*)c; *r=k->snap.r[k->pos].rowid; return SQLITE_OK; }
static int xUpdate(sqlite3_vtab *t, int argc, sqlite3_value **argv, sqlite3_int64 *out){
  char m[200];
  if(argc==1){ sqlite3_int64 id=sqlite3_value_int64(argv[0]); int i=find(id); snprintf(m,sizeof m,"delete rowid=%lld",id); logm(m);
    if(i>=0){ sqlite3_value_free(rows.r[i].a); sqlite3_value_free(rows.r[i].b); rows.r[i]=rows.r[--rows.n]; } return SQLITE_OK; }
  sqlite3_int64 newid;
  if(sqlite3_value_type(argv[1])==SQLITE_NULL){ newid=0; for(int i=0;i<rows.n;i++) if(rows.r[i].rowid>newid) newid=rows.r[i].rowid; newid++; }
  else newid=sqlite3_value_int64(argv[1]);
  if(sqlite3_value_type(argv[0])==SQLITE_NULL){
    snprintf(m,sizeof m,"insert rowid=%lld a=%s b=%s",newid,sqlite3_value_text(argv[2])?(const char*)sqlite3_value_text(argv[2]):"NULL",sqlite3_value_text(argv[3])?(const char*)sqlite3_value_text(argv[3]):"NULL"); logm(m);
    rows.r=realloc(rows.r,sizeof(Row)*(rows.n+1)); rows.r[rows.n].rowid=newid; rows.r[rows.n].a=sqlite3_value_dup(argv[2]); rows.r[rows.n].b=sqlite3_value_dup(argv[3]); rows.n++; *out=newid; return SQLITE_OK; }
  sqlite3_int64 old=sqlite3_value_int64(argv[0]); int i=find(old);
  snprintf(m,sizeof m,"update rowid=%lld new_rowid=%lld a=%s b=%s",old,newid,sqlite3_value_text(argv[2])?(const char*)sqlite3_value_text(argv[2]):"NULL",sqlite3_value_text(argv[3])?(const char*)sqlite3_value_text(argv[3]):"NULL"); logm(m);
  if(i>=0){ sqlite3_value_free(rows.r[i].a); sqlite3_value_free(rows.r[i].b); rows.r[i].rowid=newid; rows.r[i].a=sqlite3_value_dup(argv[2]); rows.r[i].b=sqlite3_value_dup(argv[3]); }
  return SQLITE_OK; }
static int xBegin(sqlite3_vtab *t){ logm("begin"); if(have_before) rows_free(&before); before=rows_dup(&rows); have_before=1; return SQLITE_OK; }
static int xSync(sqlite3_vtab *t){ return SQLITE_OK; }
static int xCommit(sqlite3_vtab *t){ logm("commit"); if(have_before){ rows_free(&before); have_before=0; } return SQLITE_OK; }
static int xRollback(sqlite3_vtab *t){ logm("rollback"); if(have_before){ rows_free(&rows); rows=before; before.r=0; before.n=0; have_before=0; } return SQLITE_OK; }

static sqlite3_module mod = { 1, xCreateL, xCreate, xBestIndex, xDisconnect, xDestroy, xOpen, xClose, xFilter, xNext, xEof, xColumn, xRowid, xUpdate, xBegin, xSync, xCommit, xRollback, 0, 0, 0,0,0,0 };

static sqlite3 *conns[3];
static sqlite3 *conn(int i, const char *path){ if(!conns[i]){ sqlite3_open(path,&conns[i]); sqlite3_create_module(conns[i],"memvt",&mod,0);} return conns[i]; }

int main(int argc, char **argv){
  if(argc<3){ fprintf(stderr,"usage: memvt_sqlite DB SCRIPT\n"); return 2; }
  remove(argv[1]); char wal[1024]; snprintf(wal,sizeof wal,"%s-wal",argv[1]); remove(wal);
  printf("SQLite %s\n", sqlite3_libversion());
  FILE *f=fopen(argv[2],"r"); char line[4096];
  while(fgets(line,sizeof line,f)){
    line[strcspn(line,"\n")]=0; if(!line[0]||line[0]=='#') continue;
    int ci=0; char *sql=line; const char *name="a";
    if(!strncmp(line,"@b ",3)){ ci=1; sql=line+3; name="b"; } else if(!strncmp(line,"@c ",3)){ ci=2; sql=line+3; name="c"; }
    sqlite3 *db=conn(ci,argv[1]); sqlite3_stmt *st; printf("%s> %s\n",name,sql); fflush(stdout);
    int rc=sqlite3_prepare_v2(db,sql,-1,&st,0); if(rc){ printf("  error: %s\n",sqlite3_errmsg(db)); continue; }
    while((rc=sqlite3_step(st))==SQLITE_ROW){ printf("  "); for(int i=0;i<sqlite3_column_count(st);i++){ const unsigned char *t=sqlite3_column_text(st,i); printf("%s%s",i?"|":"",t?(const char*)t:"NULL"); } printf("\n"); }
    if(rc!=SQLITE_DONE) printf("  error: %s\n",sqlite3_errmsg(db));
    sqlite3_finalize(st); fflush(stdout); fflush(stderr);
  }
  return 0; }
