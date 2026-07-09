/* Smoke test for the raw C API. Exits 0 on success, prints the failure and
 * exits 1 otherwise.
 *
 *   cl /W3 test_powder.c /I ../include /link powder_ffi.dll.lib
 */
#include <stdio.h>
#include <string.h>
#include "../include/powder.h"

static int checks = 0;

#define CHECK(cond, what)                                          \
    do {                                                           \
        checks++;                                                  \
        if (!(cond)) {                                             \
            const char *err = powder_last_error();                 \
            fprintf(stderr, "FAILED: %s (%s)\n", what,             \
                    err ? err : "no engine error");                \
            return 1;                                              \
        }                                                          \
    } while (0)

int main(void) {
    PowderClient *db = powder_connect("sqlite::memory:");
    CHECK(db != NULL, "connect");

    CHECK(powder_execute(db, "CREATE TABLE t (id INTEGER, name TEXT, score REAL)", NULL) == 0,
          "create table");
    CHECK(powder_execute(db, "INSERT INTO t VALUES (?, ?, ?), (?, ?, ?)",
                         "[1, \"alice\", 9.5, 2, \"bob\", null]") == 2,
          "insert 2 rows");

    size_t len = 0;
    unsigned char *buf = powder_query(db, "SELECT id, name, score FROM t ORDER BY id", NULL, &len);
    CHECK(buf != NULL && len > 24, "query returns a PCB buffer");
    CHECK(memcmp(buf, "PCB1", 4) == 0, "buffer starts with the PCB magic");

    /* copy-out path used by pointer-restricted hosts */
    unsigned char first4[4];
    powder_copy_out(buf, 4, first4);
    CHECK(memcmp(first4, "PCB1", 4) == 0, "powder_copy_out copies bytes");
    powder_free_buffer(buf, len);

    /* error paths */
    CHECK(powder_query(db, "SELECT * FROM missing", NULL, &len) == NULL, "bad SQL returns NULL");
    const char *err = powder_last_error();
    CHECK(err != NULL && strstr(err, "missing") != NULL, "error mentions the table");
    unsigned char small[8];
    size_t need = powder_last_error_copy(small, sizeof small);
    CHECK(need > sizeof small, "last_error_copy reports full length");

    powder_close(db);
    printf("c binding OK (%d checks)\n", checks);
    return 0;
}
