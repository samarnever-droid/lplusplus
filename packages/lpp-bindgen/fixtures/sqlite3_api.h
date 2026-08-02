#ifndef C2LPP_SQLITE3_API_H
#define C2LPP_SQLITE3_API_H

#include <stddef.h>

#define SQLITE_API
#define SQLITE_OK 0
#define SQLITE_ERROR 1
#define SQLITE_ROW 100
#define SQLITE_DONE 101

typedef struct sqlite3 sqlite3;
typedef struct sqlite3_stmt sqlite3_stmt;
typedef void (*sqlite3_destructor_type)(void *);

SQLITE_API int sqlite3_open(const char *filename, sqlite3 **ppDb);
SQLITE_API int sqlite3_close(sqlite3 *db);
SQLITE_API const char *sqlite3_errmsg(sqlite3 *db);
SQLITE_API int sqlite3_prepare_v2(
    sqlite3 *db,
    const char *sql,
    int nByte,
    sqlite3_stmt **statement,
    const char **tail
);
SQLITE_API int sqlite3_step(sqlite3_stmt *statement);
SQLITE_API int sqlite3_finalize(sqlite3_stmt *statement);
SQLITE_API int sqlite3_bind_text(
    sqlite3_stmt *statement,
    int index,
    const char *value,
    int length,
    sqlite3_destructor_type destroy
);
SQLITE_API int sqlite3_column_int(sqlite3_stmt *statement, int column);
SQLITE_API const unsigned char *sqlite3_column_text(sqlite3_stmt *statement, int column);
SQLITE_API int sqlite3_exec(
    sqlite3 *db,
    const char *sql,
    int (*callback)(void *, int, char **, char **),
    void *context,
    char **error_message
);

#endif
