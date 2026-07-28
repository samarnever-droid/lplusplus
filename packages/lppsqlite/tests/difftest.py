#!/usr/bin/env python3
"""difftest.py — differential test: lppsqlite vs the real sqlite3.

For each case we run the same SQL through both engines and compare the
rendered output. Every case is also verified with PRAGMA integrity_check on
the file lppsqlite produced, and (where applicable) re-read by real SQLite.

Usage:  python3 difftest.py [--verbose]
"""
import os
import re
import sqlite3
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SHELL = os.path.join(ROOT, "lppsqlite")

VERBOSE = "--verbose" in sys.argv or "-v" in sys.argv


def run_lpp(db, sql):
    env = dict(os.environ)
    env.update(LPPSQLITE_DB=db, LPPSQLITE_SQL=sql,
               LPPSQLITE_HEADERS="off", LPPSQLITE_MODE="list")
    p = subprocess.run([os.path.join(ROOT, "build", "lppsqlite")],
                       env=env, capture_output=True, text=True, timeout=300)
    return p.stdout.strip()


def fmt_val(v):
    if v is None:
        return "NULL"
    if isinstance(v, float):
        # match the engine's %!.15g-style rendering
        if v == int(v) and abs(v) < 1e15:
            return f"{int(v)}.0"
        s = repr(v)
        return s
    if isinstance(v, bytes):
        return f"<blob {len(v)}>"
    return str(v)


def run_real(db, sql):
    con = sqlite3.connect(db)
    out = []
    try:
        for stmt in [s for s in sql.split(";") if s.strip()]:
            cur = con.execute(stmt)
            if cur.description:
                for row in cur.fetchall():
                    out.append("|".join(fmt_val(c) for c in row))
        con.commit()
    finally:
        con.close()
    return "\n".join(out).strip()


CASES = [
    # (name, setup SQL, query SQL)
    ("basic select",
     "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1,'x'),(2,'y'),(3,'z');",
     "SELECT * FROM t;"),
    ("where",
     "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1,'x'),(2,'y'),(3,'z');",
     "SELECT b FROM t WHERE a >= 2;"),
    ("order by desc",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (3),(1),(2);",
     "SELECT a FROM t ORDER BY a DESC;"),
    ("aggregates",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3),(4);",
     "SELECT COUNT(*), SUM(a), MIN(a), MAX(a) FROM t;"),
    ("avg",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3),(4);",
     "SELECT AVG(a) FROM t;"),
    ("group by",
     "CREATE TABLE t(g TEXT, v INTEGER); INSERT INTO t VALUES ('a',1),('a',2),('b',5);",
     "SELECT g, SUM(v) FROM t GROUP BY g ORDER BY g;"),
    ("having",
     "CREATE TABLE t(g TEXT, v INTEGER); INSERT INTO t VALUES ('a',1),('a',2),('b',5);",
     "SELECT g, COUNT(*) FROM t GROUP BY g HAVING COUNT(*) > 1;"),
    ("distinct",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(1),(2),(2),(3);",
     "SELECT DISTINCT a FROM t ORDER BY a;"),
    ("limit offset",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3),(4),(5);",
     "SELECT a FROM t ORDER BY a LIMIT 2 OFFSET 1;"),
    ("inner join",
     "CREATE TABLE a(id INTEGER, n TEXT); CREATE TABLE b(aid INTEGER, v INTEGER);"
     "INSERT INTO a VALUES (1,'x'),(2,'y'); INSERT INTO b VALUES (1,10),(1,20),(2,30);",
     "SELECT a.n, b.v FROM a JOIN b ON a.id = b.aid ORDER BY a.n, b.v;"),
    ("left join null",
     "CREATE TABLE a(id INTEGER, n TEXT); CREATE TABLE b(aid INTEGER, v INTEGER);"
     "INSERT INTO a VALUES (1,'x'),(2,'y'); INSERT INTO b VALUES (1,10);",
     "SELECT a.n FROM a LEFT JOIN b ON a.id = b.aid WHERE b.v IS NULL;"),
    ("join with aggregate",
     "CREATE TABLE a(id INTEGER, n TEXT); CREATE TABLE b(aid INTEGER, v INTEGER);"
     "INSERT INTO a VALUES (1,'x'),(2,'y'); INSERT INTO b VALUES (1,10),(1,20),(2,30);",
     "SELECT a.n, SUM(b.v) FROM a JOIN b ON a.id=b.aid GROUP BY a.n ORDER BY a.n;"),
    ("like",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('apple'),('banana'),('avocado');",
     "SELECT s FROM t WHERE s LIKE 'a%' ORDER BY s;"),
    ("like underscore",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('cat'),('cot'),('coat');",
     "SELECT s FROM t WHERE s LIKE 'c_t' ORDER BY s;"),
    ("in list",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3),(4);",
     "SELECT a FROM t WHERE a IN (2,4) ORDER BY a;"),
    ("not in",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3),(4);",
     "SELECT a FROM t WHERE a NOT IN (2,4) ORDER BY a;"),
    ("between",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(5),(10);",
     "SELECT a FROM t WHERE a BETWEEN 2 AND 9;"),
    ("is null",
     "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1,NULL),(2,'x');",
     "SELECT a FROM t WHERE b IS NULL;"),
    ("null arithmetic",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (NULL),(1);",
     "SELECT a + 1 FROM t;"),
    ("three valued and",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (NULL),(0),(1);",
     "SELECT COUNT(*) FROM t WHERE a = 1;"),
    ("string functions",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('Hello');",
     "SELECT upper(s), lower(s), length(s), substr(s,2,3) FROM t;"),
    ("trim",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('  pad  ');",
     "SELECT '['||trim(s)||']', '['||ltrim(s)||']', '['||rtrim(s)||']' FROM t;"),
    ("replace instr",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('banana');",
     "SELECT replace(s,'an','X'), instr(s,'nan') FROM t;"),
    ("abs round",
     "SELECT 1;",
     "SELECT abs(-5), abs(5), round(3.14159, 2), round(2.5);"),
    ("coalesce nullif",
     "SELECT 1;",
     "SELECT coalesce(NULL,NULL,3), ifnull(NULL,'d'), nullif(5,5), nullif(5,6);"),
    ("typeof",
     "CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB);"
     "INSERT INTO t VALUES (1,'s',1.5,NULL);",
     "SELECT typeof(a), typeof(b), typeof(c), typeof(d) FROM t;"),
    ("case expression",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(5),(10);",
     "SELECT CASE WHEN a < 3 THEN 'low' WHEN a < 8 THEN 'mid' ELSE 'high' END FROM t ORDER BY a;"),
    ("cast",
     "SELECT 1;",
     "SELECT CAST('42' AS INTEGER), CAST(3.9 AS INTEGER), CAST(42 AS TEXT);"),
    ("arithmetic",
     "SELECT 1;",
     "SELECT 2+3, 10-4, 6*7, 20/3, 20%3, -5;"),
    ("integer division",
     "SELECT 1;",
     "SELECT 7/2, 7.0/2, 1/0;"),
    ("concat",
     "CREATE TABLE t(a TEXT, b TEXT); INSERT INTO t VALUES ('foo','bar');",
     "SELECT a || b, a || 1 FROM t;"),
    ("comparison chain",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3);",
     "SELECT COUNT(*) FROM t WHERE a > 1 AND a < 3;"),
    ("or condition",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3);",
     "SELECT a FROM t WHERE a = 1 OR a = 3 ORDER BY a;"),
    ("not condition",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3);",
     "SELECT a FROM t WHERE NOT (a = 2) ORDER BY a;"),
    ("update",
     "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1,'x'),(2,'y');"
     "UPDATE t SET b = 'z' WHERE a = 1;",
     "SELECT * FROM t ORDER BY a;"),
    ("update expression",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3);"
     "UPDATE t SET a = a * 10;",
     "SELECT a FROM t ORDER BY a;"),
    ("delete",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3);"
     "DELETE FROM t WHERE a = 2;",
     "SELECT a FROM t ORDER BY a;"),
    ("delete all",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2); DELETE FROM t;",
     "SELECT COUNT(*) FROM t;"),
    ("insert select",
     "CREATE TABLE a(x INTEGER); CREATE TABLE b(x INTEGER);"
     "INSERT INTO a VALUES (1),(2),(3); INSERT INTO b SELECT x FROM a WHERE x > 1;",
     "SELECT x FROM b ORDER BY x;"),
    ("rowid alias",
     "CREATE TABLE t(id INTEGER PRIMARY KEY, s TEXT); INSERT INTO t VALUES (5,'a'),(9,'b');",
     "SELECT id, rowid FROM t ORDER BY id;"),
    ("implicit rowid",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('a'),('b');",
     "SELECT rowid, s FROM t ORDER BY rowid;"),
    ("union all",
     "CREATE TABLE a(x INTEGER); CREATE TABLE b(x INTEGER);"
     "INSERT INTO a VALUES (1),(2); INSERT INTO b VALUES (2),(3);",
     "SELECT x FROM a UNION ALL SELECT x FROM b;"),
    ("union",
     "CREATE TABLE a(x INTEGER); CREATE TABLE b(x INTEGER);"
     "INSERT INTO a VALUES (1),(2); INSERT INTO b VALUES (2),(3);",
     "SELECT x FROM a UNION SELECT x FROM b;"),
    ("text affinity",
     "CREATE TABLE t(a TEXT); INSERT INTO t VALUES (42);",
     "SELECT a, typeof(a) FROM t;"),
    ("integer affinity",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES ('42');",
     "SELECT a, typeof(a) FROM t;"),
    ("real affinity",
     "CREATE TABLE t(a REAL); INSERT INTO t VALUES (42);",
     "SELECT a, typeof(a) FROM t;"),
    ("negative numbers",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (-1),(-100),(0);",
     "SELECT a FROM t ORDER BY a;"),
    ("large integers",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (9223372036854775807),(-9223372036854775807);",
     "SELECT a FROM t ORDER BY a;"),
    ("real values",
     "CREATE TABLE t(a REAL); INSERT INTO t VALUES (1.5),(-2.25),(0.1);",
     "SELECT a FROM t ORDER BY a;"),
    ("empty string",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES (''),('x');",
     "SELECT length(s) FROM t ORDER BY length(s);"),
    ("quoted string",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('it''s');",
     "SELECT s FROM t;"),
    ("unicode",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('héllo wörld ✓');",
     "SELECT s, length(s) FROM t;"),
    ("many rows",
     "CREATE TABLE t(a INTEGER);" +
     "INSERT INTO t VALUES " + ",".join(f"({i})" for i in range(1, 401)) + ";",
     "SELECT COUNT(*), SUM(a), MIN(a), MAX(a) FROM t;"),
    ("order by expression",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2),(3);",
     "SELECT a FROM t ORDER BY -a;"),
    ("order by ordinal",
     "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (2,'x'),(1,'y');",
     "SELECT b, a FROM t ORDER BY 2;"),
    ("order by hidden column",
     "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (2,'x'),(1,'y');",
     "SELECT b FROM t ORDER BY a;"),
    ("multi column order",
     "CREATE TABLE t(a INTEGER, b INTEGER); INSERT INTO t VALUES (1,2),(1,1),(2,1);",
     "SELECT a,b FROM t ORDER BY a, b DESC;"),
    ("group concat",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('a'),('b'),('c');",
     "SELECT group_concat(s) FROM t;"),
    ("count distinct",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(1),(2);",
     "SELECT COUNT(DISTINCT a) FROM t;"),
    ("sum empty",
     "CREATE TABLE t(a INTEGER);",
     "SELECT SUM(a), COUNT(*) FROM t;"),
    ("min max text",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('pear'),('apple'),('fig');",
     "SELECT MIN(s), MAX(s) FROM t;"),
    ("nested expressions",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (5);",
     "SELECT (a + 1) * 2 - 3 FROM t;"),
    ("multiple tables comma",
     "CREATE TABLE a(x INTEGER); CREATE TABLE b(y INTEGER);"
     "INSERT INTO a VALUES (1),(2); INSERT INTO b VALUES (10),(20);",
     "SELECT x, y FROM a, b ORDER BY x, y;"),
    ("column alias",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1);",
     "SELECT a AS renamed FROM t;"),
    ("table alias",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1);",
     "SELECT x.a FROM t AS x;"),
    ("drop table",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1); DROP TABLE t;"
     "CREATE TABLE t(b TEXT); INSERT INTO t VALUES ('new');",
     "SELECT b FROM t;"),
    ("bit operations",
     "SELECT 1;",
     "SELECT 6 & 3, 6 | 3, 1 << 4, 256 >> 4;"),
    ("in subquery",
     "CREATE TABLE t(a INTEGER); CREATE TABLE u(b INTEGER);"
     "INSERT INTO t VALUES (1),(2),(3); INSERT INTO u VALUES (2),(3);",
     "SELECT a FROM t WHERE a IN (SELECT b FROM u) ORDER BY a;"),
    ("not in subquery",
     "CREATE TABLE t(a INTEGER); CREATE TABLE u(b INTEGER);"
     "INSERT INTO t VALUES (1),(2),(3); INSERT INTO u VALUES (2),(3);",
     "SELECT a FROM t WHERE a NOT IN (SELECT b FROM u) ORDER BY a;"),
    ("scalar subquery",
     "CREATE TABLE u(b INTEGER); INSERT INTO u VALUES (2),(3);",
     "SELECT (SELECT MAX(b) FROM u);"),
    ("scalar subquery in expr",
     "CREATE TABLE t(a INTEGER); CREATE TABLE u(b INTEGER);"
     "INSERT INTO t VALUES (1); INSERT INTO u VALUES (5);",
     "SELECT a + (SELECT MAX(b) FROM u) FROM t;"),
    ("exists subquery",
     "CREATE TABLE t(a INTEGER); CREATE TABLE u(b INTEGER);"
     "INSERT INTO t VALUES (1),(2); INSERT INTO u VALUES (9);",
     "SELECT a FROM t WHERE EXISTS (SELECT 1 FROM u) ORDER BY a;"),
    ("not exists subquery",
     "CREATE TABLE t(a INTEGER); CREATE TABLE u(b INTEGER);"
     "INSERT INTO t VALUES (1),(2);",
     "SELECT a FROM t WHERE NOT EXISTS (SELECT 1 FROM u) ORDER BY a;"),
    ("empty scalar subquery",
     "CREATE TABLE t(a INTEGER); CREATE TABLE u(b INTEGER); INSERT INTO t VALUES (1);",
     "SELECT (SELECT b FROM u) FROM t;"),
    ("glob",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('abc'),('abd'),('xyz');",
     "SELECT s FROM t WHERE s GLOB 'ab*' ORDER BY s;"),
    ("hex function",
     "SELECT 1;",
     "SELECT hex('AB');"),
    ("char unicode fns",
     "SELECT 1;",
     "SELECT char(72,105), unicode('A');"),
    ("printf",
     "SELECT 1;",
     "SELECT printf('%d-%s', 42, 'x');"),
    ("iif",
     "SELECT 1;",
     "SELECT iif(1>0,'y','n'), iif(0>1,'y','n');"),
    ("min max multi arg",
     "SELECT 1;",
     "SELECT min(3,1,2), max(3,1,2);"),
    ("update multiple columns",
     "CREATE TABLE t(a INTEGER, b INTEGER); INSERT INTO t VALUES (1,1);"
     "UPDATE t SET a=10, b=20;",
     "SELECT a,b FROM t;"),
    ("delete then insert reuse",
     "CREATE TABLE t(id INTEGER PRIMARY KEY, s TEXT);"
     "INSERT INTO t VALUES (1,'a'),(2,'b'),(3,'c'); DELETE FROM t WHERE id=2;"
     "INSERT INTO t VALUES (2,'new');",
     "SELECT id,s FROM t ORDER BY id;"),
    ("blob roundtrip",
     "CREATE TABLE t(b BLOB); INSERT INTO t VALUES (x'010203');",
     "SELECT hex(b), length(b), typeof(b) FROM t;"),
    ("substr negative",
     "SELECT 1;",
     "SELECT substr('abcdef', -3), substr('abcdef', 2, 3);"),
    ("nested case",
     "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1),(2);",
     "SELECT CASE a WHEN 1 THEN 'one' WHEN 2 THEN 'two' END FROM t ORDER BY a;"),
    ("where with function",
     "CREATE TABLE t(s TEXT); INSERT INTO t VALUES ('Apple'),('banana');",
     "SELECT s FROM t WHERE lower(s) LIKE 'a%';"),
    ("aggregate over join with where",
     "CREATE TABLE a(id INTEGER, n TEXT); CREATE TABLE b(aid INTEGER, v INTEGER);"
     "INSERT INTO a VALUES (1,'x'),(2,'y'); INSERT INTO b VALUES (1,10),(1,20),(2,30);",
     "SELECT COUNT(*), SUM(b.v) FROM a JOIN b ON a.id=b.aid WHERE b.v > 15;"),
]


def main():
    passed = failed = 0
    failures = []
    for name, setup, query in CASES:
        with tempfile.TemporaryDirectory() as td:
            lpp_db = os.path.join(td, "l.db")
            real_db = os.path.join(td, "r.db")

            # setup + query on lppsqlite
            run_lpp(lpp_db, setup)
            lpp_out = run_lpp(lpp_db, query)

            # setup + query on real sqlite3
            con = sqlite3.connect(real_db)
            con.executescript(setup)
            con.commit()
            con.close()
            real_out = run_real(real_db, query)

            # integrity of the lppsqlite-produced file, read by real sqlite3
            integ = "n/a"
            if os.path.exists(lpp_db):
                try:
                    c = sqlite3.connect(lpp_db)
                    integ = c.execute("PRAGMA integrity_check").fetchone()[0]
                    c.close()
                except Exception as e:  # pragma: no cover
                    integ = f"ERROR {e}"

            ok = (lpp_out == real_out) and integ == "ok"
            if ok:
                passed += 1
                if VERBOSE:
                    print(f"PASS  {name}")
            else:
                failed += 1
                failures.append((name, query, lpp_out, real_out, integ))
                print(f"FAIL  {name}")
                print(f"      query : {query}")
                print(f"      lpp   : {lpp_out!r}")
                print(f"      real  : {real_out!r}")
                print(f"      integ : {integ}")

    total = passed + failed
    print()
    print(f"differential: {passed}/{total} cases match real SQLite "
          f"(and produce integrity-clean files)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
