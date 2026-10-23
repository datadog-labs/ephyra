# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Tests of pretriage.py: `cd deploy && python3 -m unittest` (stdlib only).

The parsers (Rust Debug values, the oracles' messages, the CLI's quote
mode), the comparison modes, the verdict logic on stub CLIs, and that the
skip lists match the generator's sqlancer.rs (skipped when it is not at
../generator, or EPHYRA_SQLANCER names it)."""

import json
import os
import re
import shutil
import tempfile
import unittest

import pretriage as P

NUL = P.NULL


def i(v):
    return ("i", v)


def r(v):
    return ("r", v)


def t(s):
    return ("t", s.encode())


def b(h):
    return ("b", bytes.fromhex(h))


class DebugValues(unittest.TestCase):
    def test_rows_of_every_kind(self):
        o = P.parse_outcome('Ok([[NULL, -1, -45.0, 84.25, -9.223372036854776e18, 1e16, inf, -inf, NaN, "a", x\'\', x\'61bd\', jsonb("{\\"a\\":1}")], []])')
        self.assertEqual(o[0], "ok")
        row = o[1][0]
        self.assertEqual(row[:4], [NUL, i(-1), r(-45.0), r(84.25)])
        self.assertEqual(row[4], r(-9.223372036854776e18))
        self.assertEqual(row[5], r(1e16))
        self.assertEqual(row[6], r(float("inf")))
        self.assertEqual(row[7], r(float("-inf")))
        self.assertTrue(row[8][1] != row[8][1])
        self.assertEqual(row[9:12], [t("a"), b(""), b("61bd")])
        self.assertEqual(row[12], ("j", b'{"a":1}', b""))
        self.assertEqual(o[1][1], [])

    def test_text_escapes(self):
        o = P.parse_outcome('Ok([["x\\ny\\t\\"q\\" \\\\ \\u{301}é😀 ж", "{\\n    \\"0\\": 1E-2\\n}"]])')
        self.assertEqual(o[1][0][0], ("t", 'x\ny\t"q" \\ ́é😀 ж'.encode()))
        self.assertEqual(o[1][0][1], ("t", b'{\n    "0": 1E-2\n}'))

    def test_errors_and_empty(self):
        self.assertEqual(P.parse_outcome('Err("internal query planner error")'), ("err", "internal query planner error"))
        self.assertEqual(P.parse_outcome('Err("near \\"x\\": syntax error")'), ("err", 'near "x": syntax error'))
        self.assertEqual(P.parse_outcome("Ok([])"), ("ok", []))
        self.assertEqual(P.parse_row("[NULL]"), [NUL])
        self.assertEqual(P.parse_row("[x'']"), [b("")])
        with self.assertRaises(P.Unparsed):
            P.parse_outcome("Ok([[1, ]])")
        with self.assertRaises(P.Unparsed):
            P.parse_outcome("Maybe")


class Messages(unittest.TestCase):
    def test_tlp_partitions_and_newlines_in_literals(self):
        osql = "SELECT json_set('[ \t false,{\"a\":\n1},\n{\"b\":2}]', '$[0]', 1), c0 FROM t4 AS a0"
        parts = [osql + " WHERE ((c0 = 'a\nb'))", osql + " WHERE NOT ((c0 = 'a\nb'))", osql + " WHERE ((c0 = 'a\nb')) IS NULL"]
        msg = ("results differ (tlp-where)\n  original: " + osql + "\n    -> Ok([[\"x\", 1]])\n  variant:  "
               + (";\n" + " " * 13).join(parts) + "\n    -> Ok([[\"x\", 1], [\"y\", 2]])")
        m = P.parse_message(msg)
        self.assertEqual(m["kind"], "differ")
        self.assertEqual(m["variant"], "tlp-where")
        self.assertEqual(m["osql"], osql)
        self.assertEqual(m["vstmts"], parts)
        self.assertEqual(P.parse_outcome(m["var"])[1][1], [t("y"), i(2)])

    def test_one_side_fails(self):
        msg = ("the variant fails, the original does not (tlp-agg)\n  original: SELECT count(*) FROM t0 AS a0\n    -> Ok([[0]])\n"
               "  variant:  SELECT sum(ref0) FROM (SELECT count(*) AS ref0 FROM t0 AS a0 WHERE (1) UNION ALL SELECT 2)\n"
               '    -> Err("internal query planner error")')
        m = P.parse_message(msg)
        self.assertEqual(m["why"], "the variant fails, the original does not")
        self.assertEqual(m["vstmts"], ["SELECT sum(ref0) FROM (SELECT count(*) AS ref0 FROM t0 AS a0 WHERE (1) UNION ALL SELECT 2)"])
        self.assertEqual(P.parse_outcome(m["var"]), ("err", "internal query planner error"))

    def test_failing_queries(self):
        m = P.parse_message("the original query fails: SELECT 'a\n  -> b' FROM t0 WHERE x\n  -> malformed JSON")
        self.assertEqual((m["kind"], m["what"], m["sql"], m["error"]), ("fails", "original", "SELECT 'a\n  -> b' FROM t0 WHERE x", "malformed JSON"))
        m = P.parse_message("a SELECT of the compound fails: SELECT 1\n  -> malformed JSON\n")
        self.assertEqual((m["what"], m["sql"]), ("compound-select", "SELECT 1"))
        m = P.parse_message("the window's input fails: SELECT 2\n  -> LIKE or GLOB pattern too complex")
        self.assertEqual((m["what"], m["error"]), ("window-input", "LIKE or GLOB pattern too complex"))

    def test_out_of_order(self):
        m = P.parse_message("rows 0 and 1 out of order (order-sorted)\n  query: WITH x0 AS (SELECT c2 AS c0 FROM t5 AS a0 ORDER BY a0.c2 LIMIT 3 OFFSET 0) "
                            "SELECT c0 FROM x0 AS a0 GROUP BY c0 ORDER BY 1 DESC\n    -> [NULL]\n    -> [x'']")
        self.assertEqual(m["kind"], "order")
        self.assertEqual([P.parse_row(x) for x in m["rows"]], [[NUL], [b("")]])
        self.assertEqual(P.Shape(m["sql"]).order, [(("col", 0), True, False)])

    def test_computed_sides(self):
        m = P.parse_message("results differ (order-limit)\n  original: SELECT * FROM t0 ORDER BY 1 (its rows 2.., 3 of them)\n    -> Ok([[1]])\n"
                            "  variant:  SELECT * FROM t0 ORDER BY 1 LIMIT 3 OFFSET 2\n    -> Ok([[2]])")
        self.assertEqual((m["osql"], m["slice"]), ("SELECT * FROM t0 ORDER BY 1", (2, 3)))
        m = P.parse_message("results differ (setops-def)\n  original: SELECT 1 UNION SELECT 2 (by definition, from its SELECTs' rows)\n    -> Ok([[1], [2]])\n"
                            "  variant:  SELECT 1 UNION SELECT 2\n    -> Ok([[1]])")
        self.assertEqual(m["expected_side"], "orig")
        m = P.parse_message("results differ (window-def)\n  original: SELECT rank() OVER () FROM t0\n    -> Ok([[1]])\n"
                            "  variant:  SELECT NULL FROM t0 (the windows computed from its rows)\n    -> Ok([[2]])")
        self.assertEqual(m["expected_side"], "var")

    def test_noopt_optimizations_line(self):
        """plans-noopt's last line is the mask its variant ran under; the
        variant is the original's SQL."""
        q = "SELECT count(*) OVER (ORDER BY c0) FROM t1 AS a0"
        msg = (f"results differ (plans-noopt-BloomFilter)\n  original: {q}\n    -> Ok([[2], [2], [3]])\n  variant:  {q}\n"
               "    -> Ok([[1], [3], [3]])\n  optimizations off: 0x00080000 (BloomFilter)\n")
        m = P.parse_message(msg)
        self.assertEqual((m["variant"], m["osql"], m["vstmts"], m["opts"], m["opts_names"]), ("plans-noopt-BloomFilter", q, [q], 0x80000, "BloomFilter"))
        self.assertEqual(P.parse_outcome(m["var"])[1], [[i(1)], [i(3)], [i(3)]])
        m = P.parse_message(msg.replace("(plans-noopt-BloomFilter)", "(plans-noopt)").replace(
            "0x00080000 (BloomFilter)", "0xfdffffff (all but Coroutines; alone, each of PropagateConst 0x00008000, BloomFilter 0x00080000)"))
        self.assertEqual((m["variant"], m["opts"]), ("plans-noopt", 0xfdffffff))
        self.assertTrue(m["opts_names"].startswith("all but Coroutines"))
        # Without its mask a plans-noopt message cannot be replayed; other variants have none.
        with self.assertRaises(P.Unparsed):
            P.parse_message(msg.replace("\n  optimizations off: 0x00080000 (BloomFilter)", ""))
        self.assertNotIn("opts", P.parse_message(msg.replace("(plans-noopt-BloomFilter)", "(plans-noindex)").replace("\n  optimizations off: 0x00080000 (BloomFilter)", "")))
        self.assertEqual([P.noopt(v) for v in ("plans-noopt", "plans-noopt-Stat4", "plans-noindex", "plans-not")], [True, True, False, False])
        self.assertEqual(P.infer_mode("plans-noopt-BloomFilter", q + " ORDER BY 1", q + " ORDER BY 1"), "ordered")

    def test_other_kinds_are_unparsed(self):
        for text in ("panic: index out of bounds", "statement 3: CREATE TABLE t0(c0): table t0 already exists", "results differ (x)\n  original: SELECT 1"):
            with self.assertRaises(P.Unparsed):
                P.parse_message(text)


class QuoteMode(unittest.TestCase):
    def test_values(self):
        out = (b"1,1.0,0.0,1.0e+300,-9.2233720368547758e+18,Inf,-Inf,'a''b',unistr('x\\u000ay'),x'00ff',NULL,'',"
               b"'\xc3\xa9',123456789012345678,unistr('a\\\\b\\u000a'),'\xffA','a,b'\n2.5e-10,0.33333333333333332\n")
        rows = P.parse_quote(out)
        self.assertEqual(rows[0][:7], [i(1), r(1.0), r(0.0), r(1e300), r(-9.223372036854775808e18), r(float("inf")), r(float("-inf"))])
        self.assertEqual(rows[0][7:], [t("a'b"), t("x\ny"), b("00ff"), NUL, t(""), t("é"), i(123456789012345678), t("a\\b\n"), ("t", b"\xffA"), t("a,b")])
        self.assertEqual(rows[1], [r(2.5e-10), r(1 / 3)])

    def test_old_shell_concatenation(self):
        self.assertEqual(P.parse_quote(b"'a'||char(10)||'b'\n"), [[t("a\nb")]])

    def test_errors(self):
        self.assertEqual(P.cli_error(b"Parse error near line 4: internal query planner error (2)\n"), "internal query planner error")
        self.assertEqual(P.cli_error(b"Parse error near line 6: ambiguous column name: c0\n  SELECT * FROM t0\n         ^--- error here\n"), "ambiguous column name: c0")
        self.assertEqual(P.cli_error(b"Error near line 15: FOREIGN KEY constraint failed\n"), "FOREIGN KEY constraint failed")
        self.assertEqual(P.cli_error(b"Runtime error near line 2: integer overflow\n"), "integer overflow")
        self.assertIsNone(P.cli_error(b""))


class Modes(unittest.TestCase):
    def test_inferred_modes(self):
        m = P.infer_mode
        self.assertEqual(m("norec-join", "SELECT count(*) FROM t0", "SELECT sum(ref0) FROM (SELECT 1 AS ref0)"), "scalar")
        self.assertEqual(m("tlp-agg", "SELECT sum(CAST(c0 AS INTEGER)) FROM t0", ""), "sum")
        self.assertEqual(m("tlp-agg", "SELECT max(c0) FROM t0", ""), "scalar")
        self.assertEqual(m("tlp-where", "SELECT * FROM t1 AS a0", ""), "multiset")
        self.assertEqual(m("tlp-distinct", "SELECT DISTINCT * FROM t1", ""), "set")
        self.assertEqual(m("tlp-having", "SELECT c0 FROM t0 GROUP BY c0", ""), "groups")
        self.assertEqual(m("tlp-union", "SELECT c0 FROM t0 GROUP BY c0", "SELECT c0 FROM t0 WHERE (1) GROUP BY c0 UNION SELECT c0 FROM t0 WHERE NOT (1) GROUP BY c0"), "set")
        self.assertEqual(m("tlp-union", "WITH x0 AS (SELECT 1 AS c0 UNION SELECT 2) SELECT * FROM x0", "WITH x0 AS (SELECT 1 AS c0 UNION SELECT 2) SELECT * FROM x0 WHERE (1) UNION ALL SELECT * FROM x0 WHERE NOT (1)"), "multiset")
        self.assertEqual(m("plans-reorder", "WITH x0 AS (SELECT c1 FROM t0 ORDER BY 1 DESC LIMIT 5) SELECT a0.c0 FROM x0 AS a0 CROSS JOIN t3 AS a1 GROUP BY a0.c0 ORDER BY 1", ""), "ordered")
        self.assertEqual(m("cte-materialized", "WITH x0 AS (SELECT c0 FROM t0) SELECT * FROM t0 AS a0 CROSS JOIN x0 AS a3", ""), "multiset")
        self.assertEqual(m("setops-swap", "SELECT c0 FROM t0 UNION ALL SELECT c1 FROM t1", ""), "multiset")
        self.assertEqual(m("setops-swap", "SELECT c0 FROM t0 INTERSECT SELECT c1 FROM t1", ""), "equiv")
        self.assertEqual(m("setops-swap", "SELECT DISTINCT c0 FROM t0 UNION ALL SELECT c1 FROM t1", ""), "equiv")
        self.assertEqual(m("plans-noindex", "SELECT max(c0) OVER (ORDER BY c1) FROM t0", ""), "multiset")
        self.assertEqual(m("plans-noindex", "SELECT count(*), 'ORDER BY 1' FROM t0 WHERE c0 IN (SELECT c0 FROM t1 ORDER BY 1)", ""), "multiset")
        self.assertEqual(m("order-multiset", "SELECT * FROM t0 ORDER BY 1", "SELECT * FROM t0"), "multiset")
        self.assertEqual(m("setops-def", "SELECT 1 UNION SELECT 2 ORDER BY 1 (by definition)", "SELECT 1 UNION SELECT 2 ORDER BY 1"), "ordered")

    def test_compare(self):
        c = P.compare_rows
        self.assertTrue(c("multiset", [[i(1)], [i(2)]], [[i(2)], [i(1)]]))
        self.assertFalse(c("multiset", [[i(1)]], [[r(1.0)]]))
        self.assertFalse(c("multiset", [[i(1)]], [[i(1)], [i(1)]]))
        self.assertTrue(c("set", [[i(1)], [r(1.0)]], [[i(1)]]))
        self.assertTrue(c("scalar", [[i(3)]], [[r(3.0)]]))
        self.assertFalse(c("scalar", [[i(3)]], [[i(3)], [i(3)]]))
        self.assertTrue(c("sum", [[r(1.5)]], [[r(2.5)]]))
        self.assertFalse(c("sum", [[i(1)]], [[r(1.0)]]))
        self.assertTrue(c("groups", [[r(-0.0)]], [[r(0.0)]]))
        self.assertFalse(c("multiset", [[r(-0.0)]], [[r(0.0)]]))
        self.assertTrue(c("ordered", [[i(1)], [i(2)]], [[r(1.0)], [i(2)]]))
        self.assertFalse(c("ordered", [[i(1)], [i(2)]], [[i(2)], [i(1)]]))
        self.assertTrue(c("multiset", [[("j", b"[]", b"\x0b")]], [[("j", b"[]", b"\x0c")]]))

    def test_agree(self):
        self.assertEqual(P.agree("multiset", ("err", "x"), ("err", "y")), (True, "both fail"))
        self.assertEqual(P.agree("multiset", ("ok", []), ("err", "malformed JSON"))[0], True)
        self.assertEqual(P.agree("multiset", ("ok", []), ("err", "integer overflow"))[0], True)
        self.assertEqual(P.agree("multiset", ("ok", []), ("err", "internal query planner error")), (False, "the variant fails"))
        self.assertEqual(P.agree("multiset", ("ok", []), ("timeout", None))[0], None)

    def test_order(self):
        asc = [(("col", 0), False, True)]
        desc = [(("col", 0), True, False)]
        self.assertIsNone(P.out_of_order([[NUL], [i(1)], [r(1.5)], [t("a")], [b("00")]], asc))
        self.assertEqual(P.out_of_order([[NUL], [b("")]], desc), 0)
        self.assertIsNone(P.out_of_order([[b("")], [NUL]], desc))
        self.assertEqual(P.out_of_order([[i(2)], [i(1)]], asc), 0)
        # A key that is not a result column is appended as the last column.
        terms = P.Shape("SELECT c0, (c1 + 1) FROM t0 ORDER BY (c1 + 1) DESC NULLS FIRST, 1").order
        self.assertEqual(terms, [(("expr", "( c1 + 1 )"), True, True), (("col", 0), False, True)])
        self.assertIsNone(P.out_of_order([[i(1), i(9)], [i(2), i(9)], [i(0), i(3)]], terms))
        self.assertEqual(P.out_of_order([[i(2), i(9)], [i(1), i(9)]], terms), 0)

    def test_text_patterns(self):
        pats, tables = P.text_patterns("SELECT * FROM t0 AS a0 WHERE (c4 GLOB 'é*') AND c1 LIKE 'a%é' AND c2 NOT LIKE '_é' "
                                       "AND EXISTS (SELECT 1 FROM t12 AS b0 WHERE c0 GLOB x'c3a9')")
        self.assertEqual((pats, tables), (["é*"], ["t0", "t12"]))
        self.assertTrue(P.invalid_text(bytes.fromhex("fba9"), "UTF-8"))
        self.assertFalse(P.invalid_text("é".encode(), "UTF-8"))
        self.assertTrue(P.invalid_text(b"\x00\xd8", "UTF-16le"))
        self.assertFalse(P.invalid_text("é".encode("utf-16-be"), "UTF-16be"))

    def test_skip_classes(self):
        self.assertEqual(P.skip_class("malformed JSON"), "json")
        self.assertEqual(P.skip_class("LIKE or GLOB pattern too complex"), "limit")
        self.assertEqual(P.skip_class("integer overflow"), "overflow")
        self.assertEqual(P.skip_class("database or disk is full"), "io")
        self.assertEqual(P.skip_class("disk I/O error"), "io")
        self.assertIsNone(P.skip_class("internal query planner error"))


class StubReplayer:
    """Outcomes by CLI name and SQL, or by CLI name, SQL and mode, or (with
    optimizations off) by CLI name, SQL and ("opts", mask); records the
    modes and masks it was asked for."""

    def __init__(self, table):
        self.table = table
        self.modes = []
        self.opts = []

    def new_example(self):
        self.case = None

    def new_signature(self):
        pass

    def set_case(self, script, before=None, readonly=False, wal=None):
        self.case = {"script": script, "before": before, "readonly": readonly, "wal": wal}

    def run(self, cli, db, digest, sql, mode="file", opts=None):
        self.modes.append((cli.name, sql, mode))
        if opts is not None:
            self.opts.append((cli.name, sql, opts))
            return self.table.get((cli.name, sql, ("opts", opts)), ("ok", []))
        return self.table.get((cli.name, sql, mode), self.table.get((cli.name, sql), ("ok", [])))

    def run_all(self, cli, db, digest, stmts, mode="file", opts=None):
        rows = []
        for s in stmts:
            o = self.run(cli, db, digest, s, mode, opts)
            if o[0] != "ok":
                return o
            rows += o[1]
        return ("ok", rows)

    def jsonb(self, cli, outcomes):
        return outcomes


class Verdicts(unittest.TestCase):
    MSG = "the original query fails: SELECT 1\n  -> internal query planner error"

    def triage(self, table, target="aaaaaaaaaaaa", target_cli=False, fix=True):
        reg = {"bugs": [{"name": "bug-a", "status": "open", "fix": {"path": "x"}}, {"name": "bug-b", "status": "open", "fix": {"path": "y"}},
                        {"name": "bug-m", "status": "open", "fix": None, "match": {"error": "ambiguous column name", "sql_all": [r"\*"]}}]}
        trunk = P.Cli("trunk", "/trunk", "bbbbbbbbbbbb")
        patched = {"bug-a": P.Cli("bug-a", "/a", "bbbbbbbbbbbb"), "bug-b": P.Cli("bug-b", "/b", "bbbbbbbbbbbb")}
        tri = P.Triage(trunk, patched, reg, "/nonexistent", StubReplayer(table))
        if target_cli:
            tri.target_cli = lambda t: P.Cli("target", "/t", t)
        return tri

    def decide(self, tri, msg, target="aaaaaaaaaaaa"):
        m = P.parse_message(msg)
        return tri._fails(m, None, "-", target) if m["kind"] == "fails" else tri._differ(m, None, "-", target)

    def test_known_new_and_several(self):
        err = ("err", "internal query planner error")
        tri = self.triage({("trunk", "SELECT 1"): err, ("bug-b", "SELECT 1"): err})
        v = self.decide(tri, self.MSG)
        self.assertEqual((v["verdict"], v["bugs"]), ("known", ["bug-a"]))
        tri = self.triage({("trunk", "SELECT 1"): err})
        self.assertEqual(self.decide(tri, self.MSG)["bugs"], ["bug-a", "bug-b"])
        tri = self.triage({("trunk", "SELECT 1"): err, ("bug-a", "SELECT 1"): err, ("bug-b", "SELECT 1"): err})
        self.assertEqual(self.decide(tri, self.MSG)["verdict"], "new")

    def test_reindex_bug_needs_trunk_to_fail_after_its_own_reindex(self):
        err = ("err", "internal query planner error")
        reg = {"bugs": [{"name": "bug-a", "status": "open", "fix": {"path": "x"}, "reindex": True}, {"name": "bug-b", "status": "open", "fix": {"path": "y"}}]}
        trunk = P.Cli("trunk", "/trunk", "bbbbbbbbbbbb")
        patched = {"bug-a": P.Cli("bug-a", "/a", "bbbbbbbbbbbb", reindex=True), "bug-b": P.Cli("bug-b", "/b", "bbbbbbbbbbbb")}
        # Trunk still fails on its own rebuilt copy: the fix is what changed it.
        tri = P.Triage(trunk, patched, reg, "/nonexistent", StubReplayer({("trunk", "SELECT 1"): err, ("trunk-reindex", "SELECT 1"): err, ("bug-b", "SELECT 1"): err}))
        v = self.decide(tri, self.MSG)
        self.assertEqual((v["verdict"], v["bugs"]), ("known", ["bug-a"]))
        # The rebuild alone makes it go away: not credited to the fix.
        tri = P.Triage(trunk, patched, reg, "/nonexistent", StubReplayer({("trunk", "SELECT 1"): err, ("bug-b", "SELECT 1"): err}))
        v = self.decide(tri, self.MSG)
        self.assertEqual((v["verdict"], v["bugs"]), ("new", []))

    def test_error_match_without_fix(self):
        err = ("err", "ambiguous column name: c0")
        tri = self.triage({(c, "SELECT * FROM t"): err for c in ("trunk", "bug-a", "bug-b")})
        v = self.decide(tri, "the original query fails: SELECT * FROM t\n  -> ambiguous column name: c0")
        self.assertEqual((v["verdict"], v["bugs"]), ("known", ["bug-m"]))

    def test_excluded_and_not_reproduced(self):
        tri = self.triage({})
        self.assertEqual(self.decide(tri, "the original query fails: SELECT 1\n  -> malformed JSON")["verdict"], "excluded")
        # Trunk does not reproduce it: the trunk CLI is the target's.
        self.assertEqual(self.decide(tri, self.MSG, target="bbbbbbbbbbbb")["verdict"], "not-reproducible")
        # A newer trunk, no CLI of the target: fixed upstream, unconfirmed.
        v = self.decide(tri, self.MSG)
        self.assertEqual(v["verdict"], "fixed-upstream")
        self.assertIsNone(v["evidence"]["target_cli"])
        # The target's CLI reproduces it, trunk does not.
        tri = self.triage({("target", "SELECT 1"): ("err", "internal query planner error")}, target_cli=True)
        self.assertEqual(self.decide(tri, self.MSG)["verdict"], "fixed-upstream")
        tri = self.triage({}, target_cli=True)
        self.assertEqual(self.decide(tri, self.MSG)["verdict"], "not-reproducible")

    def test_comparison(self):
        msg = ("results differ (norec-join)\n  original: SELECT count(*) FROM t0 WHERE p\n    -> Ok([[0]])\n"
               "  variant:  SELECT sum(ref0) FROM (SELECT (p) IS TRUE AS ref0 FROM t0)\n    -> Ok([[1]])")
        o, v = "SELECT count(*) FROM t0 WHERE p", "SELECT sum(ref0) FROM (SELECT (p) IS TRUE AS ref0 FROM t0)"
        tri = self.triage({("trunk", o): ("ok", [[i(0)]]), ("trunk", v): ("ok", [[i(1)]]),
                           ("bug-a", o): ("ok", [[i(1)]]), ("bug-a", v): ("ok", [[i(1)]]),
                           ("bug-b", o): ("ok", [[i(0)]]), ("bug-b", v): ("ok", [[NUL]])})
        res = self.decide(tri, msg)
        # bug-b: a NULL sum is 0, so 0 against 0 agrees too.
        self.assertEqual((res["verdict"], res["bugs"], res["mode"]), ("known", ["bug-a", "bug-b"], "scalar"))

    def test_storage_compares_memory_with_a_wal_file(self):
        """A storage finding: the original replayed in memory, the variant
        (the same SQL) on a WAL-mode copy, judged as any comparison."""
        q = "SELECT c0 FROM t0 AS a0"
        msg = (f"results differ (storage)\n  original: {q}\n    -> Ok([[1], [2]])\n  variant:  {q}\n    -> Ok([[1]])")
        tri = self.triage({("trunk", q, "memory"): ("ok", [[i(1)], [i(2)]]), ("trunk", q, "wal"): ("ok", [[i(1)]]),
                           ("bug-a", q, "memory"): ("ok", [[i(2)], [i(1)]]), ("bug-a", q, "wal"): ("ok", [[i(1)], [i(2)]]),
                           ("bug-b", q, "memory"): ("ok", [[i(1)], [i(2)]]), ("bug-b", q, "wal"): ("ok", [])})
        res = self.decide(tri, msg)
        self.assertEqual((res["verdict"], res["bugs"], res["mode"]), ("known", ["bug-a"], "multiset"))
        self.assertEqual({m for (c, _, m) in tri.r.modes if c == "trunk"}, {"memory", "wal"})
        # Both sides alike on trunk and the target: not reproducible.
        same = self.triage({("trunk", q, "memory"): ("ok", [[i(1)]]), ("trunk", q, "wal"): ("ok", [[i(1)]])})
        self.assertEqual(self.decide(same, msg, target="bbbbbbbbbbbb")["verdict"], "not-reproducible")
        # A side failing for the host's storage is the oracle's skip.
        io = msg.replace("Ok([[1]])", 'Err("database or disk is full")', 1).replace("Ok([[1], [2]])", "Ok([[1]])", 1)
        self.assertEqual(self.decide(self.triage({}), io)["verdict"], "excluded")

    def test_noopt_replays_its_variant_with_the_mask(self):
        """A plans-noopt finding: the original as written, the variant (the
        same SQL) with the message's mask of optimizations off, on trunk and
        on each patched CLI; bug-a's fix makes the two agree."""
        q = "SELECT c0 FROM t0 AS a0"
        msg = (f"results differ (plans-noopt-BloomFilter)\n  original: {q}\n    -> Ok([[1], [2]])\n  variant:  {q}\n"
               "    -> Ok([[1]])\n  optimizations off: 0x00080000 (BloomFilter)")
        off = ("opts", 0x80000)
        tri = self.triage({("trunk", q): ("ok", [[i(1)], [i(2)]]), ("trunk", q, off): ("ok", [[i(1)]]),
                           ("bug-a", q): ("ok", [[i(2)], [i(1)]]), ("bug-a", q, off): ("ok", [[i(1)], [i(2)]]),
                           ("bug-b", q): ("ok", [[i(1)], [i(2)]]), ("bug-b", q, off): ("ok", [[i(1)]])})
        res = self.decide(tri, msg)
        self.assertEqual((res["verdict"], res["bugs"], res["mode"]), ("known", ["bug-a"], "multiset"))
        self.assertEqual(res["evidence"]["trunk"]["optimizations_off"], "0x00080000 (BloomFilter)")
        # Every CLI ran the variant with the mask, and the original without one.
        self.assertEqual(sorted(set(tri.r.opts)), [(c, q, 0x80000) for c in ("bug-a", "bug-b", "trunk")])
        self.assertEqual(len([m for m in tri.r.modes if m[0] == "trunk"]), 2)
        # The same SQL with all optimizations on agrees: without the mask it would not reproduce.
        same = self.triage({("trunk", q): ("ok", [[i(1)], [i(2)]]), ("trunk", q, off): ("ok", [[i(2)], [i(1)]])})
        self.assertEqual(self.decide(same, msg, target="bbbbbbbbbbbb")["verdict"], "not-reproducible")

    def test_wal_examples_replay_in_wal_mode(self):
        """An example of a WAL worker (storage wal in example.json) replays
        every statement on a WAL-mode copy; a memory worker's on a plain one."""
        err = ("err", "internal query planner error")
        d = tempfile.mkdtemp(prefix="pretriage-test-")
        try:
            with open(os.path.join(d, "message.txt"), "w") as f:
                f.write(self.MSG)
            for storage, mode in (("wal", "wal"), ("memory", "file"), (None, "file")):
                with open(os.path.join(d, "example.json"), "w") as f:
                    json.dump({"target": "aaaaaaaaaaaa", "profile": "gen+wal", **({"storage": storage} if storage else {})}, f)
                tri = self.triage({("trunk", "SELECT 1", mode): err, ("bug-b", "SELECT 1", mode): err})
                v = tri.example(d)
                self.assertEqual((v["verdict"], v["bugs"], v["replay"]), ("known", ["bug-a"], mode), storage)
                self.assertEqual({m for (_, _, m) in tri.r.modes}, {mode})
        finally:
            shutil.rmtree(d)

    def test_nondeterministic_statement_is_excluded(self):
        d = tempfile.mkdtemp(prefix="pretriage-test-")
        try:
            with open(os.path.join(d, "message.txt"), "w") as f:
                f.write("statement 7: UPDATE t0 SET c0 = 'nOw': non-deterministic use of julianday() in an index")
            self.assertEqual(self.triage({}).example(d)["verdict"], "excluded")
            with open(os.path.join(d, "message.txt"), "w") as f:
                f.write("statement 3: CREATE TABLE t0(c0): table t0 already exists")
            self.assertEqual(self.triage({}).example(d)["verdict"], "manual")
        finally:
            shutil.rmtree(d)

    def test_one_side_skipped_error_is_excluded(self):
        msg = ("the variant fails, the original does not (tlp-where)\n  original: SELECT 1\n    -> Ok([[1]])\n"
               "  variant:  SELECT 1 WHERE (x);\n             SELECT 1 WHERE NOT (x);\n             SELECT 1 WHERE (x) IS NULL\n"
               '    -> Err("malformed JSON")')
        self.assertEqual(self.decide(self.triage({}), msg)["verdict"], "excluded")


class FixedTarget(unittest.TestCase):
    """A fixed run (SQLITE_TARGET=fixed): its SQLite is trunk with the known
    bugs' fixes, replayed as RUN_ROOT/clis/<target>/fixed/sqlite3."""

    MSG = Verdicts.MSG
    ERR = ("err", "internal query planner error")

    def triage(self, table, fixes=("bug-a", "bug-b"), fixed_cli=True, target_cli=False):
        tri = Verdicts().triage(table, target_cli=target_cli)
        tri.run_fixes = list(fixes)
        tri.fixed_cli = (lambda t: P.Cli("fixed", "/nonexistent/fixed/sqlite3", t)) if fixed_cli else (lambda t: None)
        return tri

    def decide(self, tri, target="bbbbbbbbbbbb"):
        return Verdicts().decide(tri, self.MSG, target=target)

    def test_fixes_only(self):
        # Trunk passes, the run's own SQLite (trunk + fixes) fails.
        v = self.decide(self.triage({("fixed", "SELECT 1"): self.ERR}))
        self.assertEqual((v["verdict"], v["fixes"]), ("fixes-only", ["bug-a", "bug-b"]))
        self.assertIn("fixed", v["evidence"])
        self.assertEqual(P.label(v), "fixes-only")
        # Neither does: not reproducible.
        v = self.decide(self.triage({}))
        self.assertEqual(v["verdict"], "not-reproducible")
        self.assertIn("fixed", v["evidence"])
        # A timeout on the fixed CLI: manual.
        self.assertEqual(self.decide(self.triage({("fixed", "SELECT 1"): ("timeout", None)}))["verdict"], "manual")

    def test_older_target(self):
        old = "aaaaaaaaaaaa"
        # The plain target reproduces it too: fixed upstream, not our fixes.
        tri = self.triage({("fixed", "SELECT 1"): self.ERR, ("target", "SELECT 1"): self.ERR}, target_cli=True)
        self.assertEqual(self.decide(tri, old)["verdict"], "fixed-upstream")
        # Only with the fixes: fixes-only.
        tri = self.triage({("fixed", "SELECT 1"): self.ERR}, target_cli=True)
        self.assertEqual(self.decide(tri, old)["verdict"], "fixes-only")
        # No CLI of the plain target: fixes-only still (as PostgreSQL's).
        v = self.decide(self.triage({("fixed", "SELECT 1"): self.ERR}), old)
        self.assertEqual(v["verdict"], "fixes-only")
        self.assertIn("no CLI of the plain target", v["reason"])
        # The run's own SQLite does not reproduce it: not reproducible, even
        # though the plain target would.
        tri = self.triage({("target", "SELECT 1"): self.ERR}, target_cli=True)
        self.assertEqual(self.decide(tri, old)["verdict"], "not-reproducible")

    def test_without_a_fixed_cli_the_plain_rules(self):
        tri = self.triage({}, fixed_cli=False)
        self.assertEqual(self.decide(tri)["verdict"], "not-reproducible")
        v = self.decide(tri, "aaaaaaaaaaaa")
        self.assertEqual(v["verdict"], "fixed-upstream")
        self.assertIsNone(v["evidence"]["target_cli"])

    def test_plain_run_ignores_the_fixed_cli(self):
        tri = self.triage({("fixed", "SELECT 1"): self.ERR}, fixes=())
        self.assertEqual(self.decide(tri)["verdict"], "not-reproducible")

    def test_known_bug_the_run_has_fixed(self):
        # bug-a's fix explains it; the run has bug-a's fix and the fixed CLI
        # still fails: the fix is incomplete.
        table = {("trunk", "SELECT 1"): self.ERR, ("bug-b", "SELECT 1"): self.ERR, ("fixed", "SELECT 1"): self.ERR}
        v = self.decide(self.triage(table))
        self.assertEqual((v["verdict"], v["bugs"], v["fix_incomplete"]), ("known", ["bug-a"], ["bug-a"]))
        self.assertIn("incomplete", v["note"])
        self.assertIn("fixed", v["evidence"])
        # The fixed CLI passes: the replay is not the run.
        table = {("trunk", "SELECT 1"): self.ERR, ("bug-b", "SELECT 1"): self.ERR}
        v = self.decide(self.triage(table))
        self.assertIn("does not reproduce", v["note"])
        # A run without bug-a's fix: no note.
        v = self.decide(self.triage(table, fixes=("bug-b",)))
        self.assertEqual(v["verdict"], "known")
        self.assertNotIn("note", v)
        # A new failure on a fixed run is new.
        table = {(c, "SELECT 1"): self.ERR for c in ("trunk", "bug-a", "bug-b")}
        self.assertEqual(self.decide(self.triage(table))["verdict"], "new")

    def test_run_fixes_and_the_unexplained_counter(self):
        root = tempfile.mkdtemp(prefix="pretriage-test-")
        try:
            run = os.path.join(root, "run")
            ex = os.path.join(run, "findings", "plans-x", "w0-1")
            os.makedirs(ex)
            self.assertEqual(P.run_fixes(run), [])
            with open(os.path.join(run, "run.json"), "w") as f:
                json.dump({"run_id": "r1", "campaign": "c", "work_tag": "t", "version_id": "v", "target_id": "bbbbbbbbbbbb",
                           "config": {"fixed_bugs": ["bug-a", "bug-b"]}}, f)
            self.assertEqual(P.run_fixes(run), ["bug-a", "bug-b"])
            with open(os.path.join(ex, "message.txt"), "w") as f:
                f.write(self.MSG)
            with open(os.path.join(ex, "example.json"), "w") as f:
                json.dump({"oracle": "plans", "profile": "gen", "storage": "memory", "target": "bbbbbbbbbbbb"}, f)
            tri = self.triage({("fixed", "SELECT 1"): self.ERR}, fixes=())
            sent = []

            class Sink:
                def count(self, name, n, tags):
                    sent.append(tags)

            class Args:
                force, budget = False, 0

            P.triage_dir(os.path.join(run, "findings"), run, tri, Args(), Sink(), "", None)
            self.assertEqual(tri.run_fixes, ["bug-a", "bug-b"])
            self.assertEqual(P.read_json(os.path.join(ex, "triage.json"))["verdict"], "fixes-only")
            self.assertIn("explained:false", sent[0].split(","))
            with open(os.path.join(run, "events.jsonl")) as f:
                self.assertEqual(json.loads(f.read().splitlines()[-1])["fixed_bugs"], ["bug-a", "bug-b"])
            with open(os.path.join(run, "triage.md")) as f:
                self.assertIn("plans-x/w0-1`: fixes-only", f.read())
        finally:
            shutil.rmtree(root)


    def test_unshrunk_input_is_not_an_example(self):
        """A statement error's example keeps the input before shrinking in
        unshrunk/ (message.txt and all): pre-triage triages the example only."""
        root = tempfile.mkdtemp(prefix="pretriage-test-")
        try:
            run = os.path.join(root, "run")
            sig = os.path.join(run, "findings", "plans-statement_N__x")
            ex = os.path.join(sig, "w0-1")
            un = os.path.join(ex, P.UNSHRUNK)
            os.makedirs(un)
            with open(os.path.join(run, "run.json"), "w") as f:
                json.dump({"run_id": "r1", "campaign": "c", "work_tag": "t", "version_id": "v", "target_id": "bbbbbbbbbbbb", "config": {}}, f)
            for d in (ex, un):
                with open(os.path.join(d, "message.txt"), "w") as f:
                    f.write(self.MSG)
                with open(os.path.join(d, "example.json"), "w") as f:
                    json.dump({"oracle": "plans", "profile": "gen", "storage": "memory", "target": "bbbbbbbbbbbb"}, f)
            self.assertEqual([e for _, e, _ in P.examples(os.path.join(run, "findings"))], ["w0-1"])
            # Even a signature dir given as the findings dir.
            self.assertEqual(list(P.examples(sig)), [])

            class Sink:
                def count(self, name, n, tags):
                    pass

            class Args:
                force, budget = False, 0

            tri = self.triage({(c, "SELECT 1"): self.ERR for c in ("trunk", "bug-a", "bug-b")})
            P.triage_dir(os.path.join(run, "findings"), run, tri, Args(), Sink(), "", None)
            self.assertTrue(os.path.exists(os.path.join(ex, "triage.json")))
            self.assertFalse(os.path.exists(os.path.join(un, "triage.json")))
            with open(os.path.join(run, "triage.jsonl")) as f:
                self.assertEqual(len(f.read().splitlines()), 1)
            with open(os.path.join(run, "triage.md")) as f:
                self.assertNotIn(P.UNSHRUNK, f.read())
        finally:
            shutil.rmtree(root)

class ReferenceSides(unittest.TestCase):
    """setops-def and window-def: the side the runner computed in Rust from
    other queries' rows is recomputed with each CLI from the same queries,
    so a fix that changes either side counts (run 20261012-0317's
    setops-def w6-406, w2-533222, w6-4567: the bug was in the left SELECT
    alone, the EXCEPT was right; they came out new)."""

    def test_compound_parts(self):
        sql = "WITH x0 AS (SELECT 1 AS c0 UNION SELECT 2) SELECT c0 FROM x0 AS a0 UNION ALL SELECT (1 + 2) FROM t0 AS a0 EXCEPT SELECT 3 ORDER BY 1 DESC, (a0.c0) LIMIT 2, 3"
        p = P.compound_parts(sql)
        w = "WITH x0 AS (SELECT 1 AS c0 UNION SELECT 2) "
        self.assertEqual(p["cores"], [w + "SELECT c0 FROM x0 AS a0", w + "SELECT (1 + 2) FROM t0 AS a0", w + "SELECT 3"])
        self.assertEqual((p["ops"], p["order"], p["limit"]), (["UNION ALL", "EXCEPT"], [(0, True, False)], (3, 2)))
        self.assertEqual(P.compound_parts("SELECT 1 UNION SELECT 2 ORDER BY 1 LIMIT -1 OFFSET 1")["limit"], (-1, 1))
        with self.assertRaises(P.Unparsed):
            P.compound_parts("SELECT 1")

    def test_set_operations(self):
        rows = P.setops_reference({"ops": ["UNION", "EXCEPT"], "order": [(0, False, True)], "limit": (2, 1)},
                                  [("ok", [[i(3)], [r(1.0)], [i(1)]]), ("ok", [[NUL], [i(2)]]), ("ok", [[i(2)]])], True)
        self.assertEqual(rows, ("ok", [[i(1)], [i(3)]]))
        # Two JSONB encodings of one value are two rows of a set.
        two = P.set_op("UNION", [[("j", b'""', b"\x07")]], [[("j", b'""', b"\x0a")]])
        self.assertEqual(len(two), 2)
        self.assertEqual(P.setops_reference({"ops": ["UNION"], "order": [], "limit": None}, [("ok", []), ("err", "x")], True), ("err", "x"))

    def triage(self, table):
        return Verdicts().triage(table)

    def decide(self, tri, msg):
        return Verdicts().decide(tri, msg)

    SETOPS = ("results differ (setops-def)\n  original: SELECT count(a1.c1 ORDER BY a0.c1) FROM t0 AS a0 JOIN t0 AS a1 EXCEPT SELECT c0 FROM t0 AS a0 (by definition, from its SELECTs' rows)\n"
              "    -> Ok([[0]])\n  variant:  SELECT count(a1.c1 ORDER BY a0.c1) FROM t0 AS a0 JOIN t0 AS a1 EXCEPT SELECT c0 FROM t0 AS a0\n    -> Ok([[1]])")
    LEFT, RIGHT, FULL = "SELECT count(a1.c1 ORDER BY a0.c1) FROM t0 AS a0 JOIN t0 AS a1", "SELECT c0 FROM t0 AS a0", "SELECT count(a1.c1 ORDER BY a0.c1) FROM t0 AS a0 JOIN t0 AS a1 EXCEPT SELECT c0 FROM t0 AS a0"

    def test_setops_bug_in_a_select_alone(self):
        """The w6-406 shape: trunk's left SELECT is wrong (0), the compound
        right (1); bug-a's fix makes the left SELECT 1. Before, only the
        compound was replayed, the same on every CLI: new."""
        table = {(c, self.FULL): ("ok", [[i(1)]]) for c in ("trunk", "bug-a", "bug-b")}
        table.update({(c, self.RIGHT): ("ok", [[i(5)]]) for c in ("trunk", "bug-a", "bug-b")})
        table.update({("trunk", self.LEFT): ("ok", [[i(0)]]), ("bug-b", self.LEFT): ("ok", [[i(0)]]), ("bug-a", self.LEFT): ("ok", [[i(1)]])})
        v = self.decide(self.triage(table), self.SETOPS)
        self.assertEqual((v["verdict"], v["bugs"]), ("known", ["bug-a"]))
        self.assertEqual(v["evidence"]["patched"]["bug-a"]["rule"], "recomputed")
        self.assertEqual(v["evidence"]["trunk"]["reference"], "Ok([[0]])")
        # Recomputed on trunk, the two sides agree: not reproduced there.
        table[("trunk", self.LEFT)] = ("ok", [[i(1)]])
        self.assertEqual(self.decide(self.triage(table), self.SETOPS)["verdict"], "fixed-upstream")

    def test_setops_compared_folded_counts_a_changed_select(self):
        """Under NOCASE or RTRIM the runner computed the compound under each
        column's sequence, which is not recomputed: the message's rows stand,
        and a fix that changes a SELECT's rows counts (rule changed-reference)."""
        msg = self.SETOPS.replace("SELECT c0 FROM t0 AS a0", "SELECT (c0 COLLATE NOCASE) FROM t0 AS a0")
        right, full = "SELECT (c0 COLLATE NOCASE) FROM t0 AS a0", self.LEFT + " EXCEPT SELECT (c0 COLLATE NOCASE) FROM t0 AS a0"
        table = {(c, full): ("ok", [[i(1)]]) for c in ("trunk", "bug-a", "bug-b")}
        table.update({(c, right): ("ok", [[i(5)]]) for c in ("trunk", "bug-a", "bug-b")})
        table.update({("trunk", self.LEFT): ("ok", [[i(0)]]), ("bug-b", self.LEFT): ("ok", [[i(0)]]), ("bug-a", self.LEFT): ("ok", [[i(1)]])})
        v = self.decide(self.triage(table), msg)
        self.assertEqual((v["verdict"], v["bugs"]), ("known", ["bug-a"]))
        self.assertEqual(v["evidence"]["patched"]["bug-a"]["rule"], "changed-reference")
        self.assertNotIn("reference", v["evidence"]["trunk"])

    def test_window_items(self):
        items = P._items("SELECT a0.c0, sum(CAST(a1.c0 AS INTEGER)) OVER (PARTITION BY a0.c1, (a0.c0 + 1) ORDER BY a1.c0 DESC, a0.rowid NULLS LAST "
                         "GROUPS BETWEEN 1 PRECEDING AND CURRENT ROW EXCLUDE TIES), rank() OVER (), count(*) OVER (ORDER BY a0.c1 ROWS 2 PRECEDING) FROM t0 AS a0, t1 AS a1")
        ws = [P.window_item(it) for it in items]
        self.assertIsNone(ws[0])
        self.assertEqual(ws[1], {"func": "sum", "arg": True, "np": 2, "order": [(True, False), (False, False)], "frame": ("GROUPS", ("p", 1), ("cur",), "ties")})
        self.assertEqual(ws[2], {"func": "rank", "arg": False, "np": 0, "order": [], "frame": None})
        self.assertEqual(ws[3], {"func": "count", "arg": False, "np": 0, "order": [(False, True)], "frame": ("ROWS", ("p", 2), ("cur",), "no others")})

    def test_window_input_changed_by_a_fix(self):
        """window-def: the windows recomputed from each CLI's rows of the
        input query; bug-a's fix changes the input (one more row), so the
        windows it gives agree with the window query's rows."""
        osql, bsql = "SELECT count(*) OVER () FROM t0 AS a0", "SELECT NULL FROM t0 AS a0"
        msg = (f"results differ (window-def)\n  original: {osql}\n    -> Ok([[2], [2]])\n  variant:  {bsql} (the windows computed from its rows)\n    -> Ok([[1]])")
        table = {(c, osql): ("ok", [[i(2)], [i(2)]]) for c in ("trunk", "bug-a", "bug-b")}
        table.update({("trunk", bsql): ("ok", [[NUL]]), ("bug-b", bsql): ("ok", [[NUL]]), ("bug-a", bsql): ("ok", [[NUL], [NUL]])})
        v = self.decide(self.triage(table), msg)
        self.assertEqual((v["verdict"], v["bugs"], v["evidence"]["patched"]["bug-a"]["rule"]), ("known", ["bug-a"], "recomputed"))

    def test_tlp_and_norec_reference_sides(self):
        """The partitions of TLP and NoREC's unoptimized count are replayed
        with each CLI: a fix that changes only them counts."""
        o, v = "SELECT count(*) FROM t0 WHERE p", "SELECT sum(ref0) FROM (SELECT (p) IS TRUE AS ref0 FROM t0)"
        msg = (f"results differ (norec)\n  original: {o}\n    -> Ok([[1]])\n  variant:  {v}\n    -> Ok([[0]])")
        table = {(c, o): ("ok", [[i(1)]]) for c in ("trunk", "bug-a", "bug-b")}
        table.update({("trunk", v): ("ok", [[i(0)]]), ("bug-b", v): ("ok", [[i(0)]]), ("bug-a", v): ("ok", [[i(1)]])})
        self.assertEqual(self.decide(self.triage(table), msg)["bugs"], ["bug-a"])
        q = "SELECT c0 FROM t0 AS a0"
        parts = [q + " WHERE (p)", q + " WHERE NOT (p)", q + " WHERE (p) IS NULL"]
        msg = ("results differ (tlp-where)\n  original: " + q + "\n    -> Ok([[1]])\n  variant:  " + P.PART_SEP.join(parts) + "\n    -> Ok([])")
        table = {(c, q): ("ok", [[i(1)]]) for c in ("trunk", "bug-a", "bug-b")}
        table.update({("bug-b", parts[0]): ("ok", [[i(1)]])})
        self.assertEqual(self.decide(self.triage(table), msg)["bugs"], ["bug-b"])

    def test_windows_compute_as_sqlite_does(self):
        """window_values against a real CLI (skipped without one): every
        function, frames of each mode, EXCLUDE, NULLs ordered both ways."""
        cli = os.environ.get("SQLITE3_CLI") or shutil.which("sqlite3")
        if not cli:
            self.skipTest("no sqlite3 CLI")
        r = P.Replayer(10)
        try:
            c = P.Cli("sys", cli, "x")
            data = "WITH t(c0, c1) AS (VALUES (1, 1), (1, 1), (1, 2), (2, NULL), (1, NULL), (2, 3), (2, 3)) "
            overs = ["PARTITION BY c0 ORDER BY c1", "ORDER BY c1 DESC NULLS FIRST", "PARTITION BY c0", "",
                     "PARTITION BY c0 ORDER BY c1 GROUPS BETWEEN 1 PRECEDING AND CURRENT ROW EXCLUDE TIES",
                     "ORDER BY c1 RANGE BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING EXCLUDE GROUP",
                     "ORDER BY c1, c0 ROWS BETWEEN 1 PRECEDING AND 2 FOLLOWING EXCLUDE CURRENT ROW", "ORDER BY c1 GROUPS 1 PRECEDING"]
            for over in overs:
                for f in ("row_number()", "rank()", "dense_rank()", "count(*)", "count(c1)", "sum(c1)", "min(c1)", "max(c1)"):
                    if "ROWS" in over or "GROUPS" in over or "RANGE" in over:
                        if f in ("row_number()", "rank()", "dense_rank()"):
                            continue
                    osql = f"{data}SELECT c0, c1, {f} OVER ({over}) FROM t"
                    w = P.window_item(P._items(osql)[2])
                    np, keys = w["np"], len(w["order"])
                    part = ["c0"] if np else []
                    order = [x.split()[0] for x in over.split("ORDER BY ")[1].split(" ROWS")[0].split(" RANGE")[0].split(" GROUPS")[0].split(", ")] if keys else []
                    arg = ["c1"] if w["arg"] else []
                    bsql = f"{data}SELECT c0, c1, NULL, {', '.join(part + order + arg) or 'NULL'} FROM t"
                    if not (part + order + arg):
                        bsql = f"{data}SELECT c0, c1, NULL FROM t"
                    got = P.windows_reference([None, None, w], 3, r.run(c, None, "d", bsql, "memory"))
                    want = r.run(c, None, "d", osql, "memory")
                    self.assertTrue(P.compare_rows("multiset", got[1], want[1]), f"{osql}: {got} against {want}")
        finally:
            r.close()


class Registry(unittest.TestCase):
    """known-bugs.json: open bugs have a check and its expected output and
    a fix or a match; retired ones record their fix; a wrong-result bug
    without a fix is matched by SQL alone."""

    REG = P.load_registry(os.path.join(P.HERE, "known-bugs.json"))

    def test_entries(self):
        names = [b["name"] for b in self.REG["bugs"]]
        self.assertEqual(len(names), len(set(names)))
        for b in self.REG["bugs"]:
            self.assertIn(b["status"], ("open", "fixed"), b["name"])
            if b["status"] == "fixed":
                # The fixing check-in, or null when it was not looked up
                # (then `how` says between which trunks it landed).
                if b["fixed_in"]["checkin"] is None:
                    self.assertIn("not looked up", b["fixed_in"]["how"], b["name"])
                else:
                    self.assertRegex(b["fixed_in"]["checkin"], r"^[0-9a-f]{10}$", b["name"])
                self.assertTrue(b["fixed_in"]["trunk"], b["name"])
                continue
            self.assertIsInstance(b["expected"], str, b["name"])
            self.assertRegex(b["expected"], r"^q1[\n|]", b["name"])
            self.assertTrue(b.get("fix") or b.get("match"), b["name"])
            for pat in (b.get("match") or {}).get("sql_all", []) + (b.get("match") or {}).get("sql_any", []):
                re.compile(pat)
        opened = {b["name"] for b in P.open_bugs(self.REG)}
        for name in ("sqlite-intersect-full-join-cte", "sqlite-groupby-subquery-orderby-direction", "sqlite-aggregate-orderby-replaces-argument",
                     "sqlite-expr-index-constant-propagation", "sqlite-json-valid-stale-cache-json5", "sqlite-bloom-filter-rowid-real-minint",
                     "sqlite-or-partial-index-covering-error", "sqlite-in-select-collate-index", "sqlite-alter-default-blob-encoding",
                     "sqlite-gencol-index-real-int", "sqlite-wr-bignull-added-column", "sqlite-group-concat-inverse-empty-text",
                     "sqlite-overflow-text-bom", "sqlite-rowvalue-in-join-reuse", "sqlite-compound-orderby-collate-groupby",
                     "sqlite-leftjoin-on-or-omitted-table-segv", "sqlite-delete-saveposition-stale-cellinfo",
                     "sqlite-balanced-merge-dedup-collation", "sqlite-temp-rename-column-twice",
                     "sqlite-vtab-create-rollback-to-savepoint", "sqlite-fts5-tokendata-missing-term-dlidx-corrupt",
                     "sqlite-rtree-update-set-rowid-ignored", "sqlite-fts5-secure-delete-dlidx-walks-next-term",
                     "sqlite-fts5-contentless-unindexed-drop-rename",
                     "sqlite-fts5-unindexed-detail-none-instances"):
            self.assertNotIn(name, opened)
        for name in ("sqlite-partial-index-constant-affinity", "sqlite-right-join-using-star-ambiguous",
                     "sqlite-propagate-const-real-minint",
                     "sqlite-wr-rowvalue-added-column", "sqlite-concat-blob-utf16-length",
                     "sqlite-wr-in-index-added-column", "sqlite-fts5-eq-constprop-aux-cursor",
                     "sqlite-fts5-match-rowid-eq-nonint",
                     "sqlite-geopoly-rowid-eq-nonint", "sqlite-geopoly-update-rowid-nochange",
                     "sqlite-fts5-locale-contentless-unindexed-insert",
                     "sqlite-fts5vocab-term-range-utf16-order", "sqlite-fts5-detail-none-secure-delete-replaced-rowid",
                     "sqlite-fts5-secure-delete-merge-drops-marker", "sqlite-fts5-near-nomatch-stale-poslist",
                     "sqlite-drop-constraint-fk-set-default", "sqlite-drop-not-null-not-deferrable",
                     "sqlite-fts5-and-nomatch-clears-later-row", "sqlite-fts5vocab-rowid-or-right-join",
                     "sqlite-fts5-trigram-like-unindexed", "sqlite-wr-multi-or-seekscan", "sqlite-rename-column-ipk-unique"):
            self.assertIn(name, opened)

    def test_propagate_const_patterns(self):
        [bug] = [b for b in self.REG["bugs"] if b["name"] == "sqlite-propagate-const-real-minint"]
        for sql, want in (("SELECT count(*) FROM t AS a0 WHERE ((a0.c1 = -9.223372036854776e18) AND (typeof(a0.c1) = 'integer'))", True),
                          ("SELECT * FROM t0 AS a0 WHERE ((-9223372036854775808.0 = a0.c2) AND ((a0.c2 || '') LIKE 'x'))", True),
                          ("SELECT * FROM t0 AS a0 WHERE ((typeof(a0.c1) = 'integer') AND (a0.c1 = -9.223372036854776e18))", True),
                          ("SELECT * FROM t0 AS a0 WHERE ((CAST(a0.c3 AS TEXT) = '') AND ((- 9.223372036854776e18) = a0.c3))", True),
                          ("SELECT * FROM t0 AS a0 WHERE ((a0.c1 = -9.223372036854776e18) AND (typeof(a0.c2) = 'integer'))", False),
                          ("SELECT * FROM t0 AS a0 WHERE ((a0.c1 = 9.223372036854776e18) AND (typeof(a0.c1) = 'integer'))", False),
                          ("SELECT * FROM t0 AS a0 WHERE ((a0.c1 = -9.223372036854776e18) AND (a0.c1 > 0))", False)):
            self.assertEqual(P.error_matches(bug, None, sql), want, sql)
        # An error bug still needs its error.
        star = {"match": {"error": "ambiguous column name", "sql_all": [r"\*"]}}
        self.assertFalse(P.error_matches(star, None, "SELECT *"))
        self.assertTrue(P.error_matches(star, "ambiguous column name: c0", "SELECT *"))

    def test_wrong_result_matched_without_a_fix(self):
        """A comparison trunk reproduces and no fix explains is the
        wrong-result bug whose patterns its SQL matches."""
        # Its patterns, as when it had no fix (it has one of ours now).
        [bug] = [dict(b, fix=None) for b in self.REG["bugs"] if b["name"] == "sqlite-propagate-const-real-minint"]
        reg = {"bugs": [bug]}
        o = "SELECT count(*) FROM t0 AS a0 WHERE ((a0.c1 = -9.223372036854776e18) AND (typeof(a0.c1) = 'integer'))"
        v = "SELECT sum(ref0) FROM (SELECT ((a0.c1 = -9.223372036854776e18) AND (typeof(a0.c1) = 'integer')) IS TRUE AS ref0 FROM t0 AS a0)"
        msg = f"results differ (norec)\n  original: {o}\n    -> Ok([[0]])\n  variant:  {v}\n    -> Ok([[1]])"
        tri = P.Triage(P.Cli("trunk", "/trunk", "bbbbbbbbbbbb"), {}, reg, "/nonexistent", StubReplayer({("trunk", o): ("ok", [[i(0)]]), ("trunk", v): ("ok", [[i(1)]])}))
        res = Verdicts().decide(tri, msg)
        self.assertEqual((res["verdict"], res["bugs"]), ("known", ["sqlite-propagate-const-real-minint"]))


class Outputs(unittest.TestCase):
    """triage.json, the triage event and the verdict counter carry the
    example's profile and storage."""

    def test_profile_and_storage_tags(self):
        root = tempfile.mkdtemp(prefix="pretriage-test-")
        try:
            run = os.path.join(root, "run")
            ex = os.path.join(run, "findings", "storage-results_differ__storage_", "w1-7")
            os.makedirs(ex)
            with open(os.path.join(run, "run.json"), "w") as f:
                json.dump({"run_id": "r1", "campaign": "c", "work_tag": "t", "version_id": "v", "target_id": "aaaaaaaaaaaa"}, f)
            with open(os.path.join(ex, "message.txt"), "w") as f:
                f.write("the original query fails: SELECT 1\n  -> internal query planner error")
            with open(os.path.join(ex, "example.json"), "w") as f:
                json.dump({"oracle": "storage", "profile": "gen+wal", "storage": "wal", "target": "aaaaaaaaaaaa"}, f)
            err = ("err", "internal query planner error")
            tri = Verdicts().triage({(c, "SELECT 1", "wal"): err for c in ("trunk", "bug-a", "bug-b")})
            sent = []

            class Sink:
                def count(self, name, n, tags):
                    sent.append((name, n, tags))

            class Args:
                force, budget = False, 0

            self.assertEqual(P.triage_dir(os.path.join(run, "findings"), run, tri, Args(), Sink(), "env:test", None), 1)
            t = P.read_json(os.path.join(ex, "triage.json"))
            self.assertEqual((t["verdict"], t["profile"], t["storage"], t["replay"]), ("new", "gen+wal", "wal", "wal"))
            with open(os.path.join(run, "events.jsonl")) as f:
                event = json.loads(f.read().splitlines()[-1])
            self.assertEqual((event["kind"], event["profile"], event["storage"]), ("triage", "gen+wal", "wal"))
            [(name, n, tags)] = sent
            self.assertEqual((name, n), ("ephyra.sqlite.triage.verdicts", 1))
            for tag in ("run:r1", "env:test", "verdict:new", "oracle:storage", "explained:false", "profile:gen+wal", "storage:wal"):
                self.assertIn(tag, tags.split(","))
        finally:
            shutil.rmtree(root)


class RealCli(unittest.TestCase):
    """The replay modes on a real sqlite3 CLI (skipped without one): a WAL
    copy is in WAL mode, `.open --deserialize` is in memory, each mode on its
    own copy, the source never touched."""

    def test_modes(self):
        cli = os.environ.get("SQLITE3_CLI") or shutil.which("sqlite3")
        if not cli:
            self.skipTest("no sqlite3 CLI")
        d = tempfile.mkdtemp(prefix="pretriage-test-")
        r = P.Replayer(10)
        try:
            db = os.path.join(d, "after.db")
            import sqlite3
            con = sqlite3.connect(db)
            con.executescript("CREATE TABLE t0(c0); INSERT INTO t0 VALUES (1), (2);")
            con.close()
            c = P.Cli("sys", cli, "x")
            modes = {m: r.run(c, db, "d", "PRAGMA journal_mode", m) for m in P.MODES}
            self.assertEqual(modes, {"file": ("ok", [[t("delete")]]), "wal": ("ok", [[t("wal")]]), "memory": ("ok", [[t("memory")]])})
            for m in P.MODES:
                self.assertEqual(r.run(c, db, "d", "SELECT sum(c0) FROM t0", m), ("ok", [[i(3)]]), m)
            with open(db, "rb") as f:
                self.assertEqual(f.read(20)[18:20], b"\x01\x01")
        finally:
            r.close()
            shutil.rmtree(d)


class RealCliOptimizations(unittest.TestCase):
    """`.testctrl optimizations MASK` reaches the replayed statement on a
    real CLI (SQLITE3_CLI, else the bugs/bin trunk CLI, else sqlite3; skipped
    without the bytecode() table or a Bloom filter in its default plan): the
    Bloom filter goes with BloomFilter off, or all, and stays with
    PropagateConst off; the CLI's own output is not taken for rows."""

    def test_mask_reaches_the_query(self):
        cli = os.environ.get("SQLITE3_CLI") or next((p for p in [os.path.join(P.ROOT_DIR, "ops", "bin", "sqlite3-trunk-f69fd5f5")] if os.access(p, os.X_OK)), None) or shutil.which("sqlite3")
        if not cli:
            self.skipTest("no sqlite3 CLI")
        d = tempfile.mkdtemp(prefix="pretriage-test-")
        r = P.Replayer(10)
        try:
            db = os.path.join(d, "after.db")
            import sqlite3
            con = sqlite3.connect(db)
            con.executescript("CREATE TABLE t1(a, b); CREATE TABLE t2(a, b); INSERT INTO t1 VALUES (1, 2); INSERT INTO t2 VALUES (1, 3);")
            con.close()
            c = P.Cli("sys", cli, "x")
            # The Bloom filter's opcodes in the join's program (the bytecode() table, as the CLI builds it).
            bloom = "SELECT count(*) FROM bytecode('SELECT * FROM t1, t2 WHERE t1.a = t2.a') WHERE opcode IN ('FilterAdd', 'Filter')"
            on = r.run(c, db, "d", bloom)
            if on != ("ok", [[i(2)]]):
                self.skipTest(f"{cli}: no bytecode() or no Bloom filter: {on}")
            for mask in (0x80000, 0xffffffff):
                self.assertEqual(r.run(c, db, "d", bloom, "file", mask), ("ok", [[i(0)]]), hex(mask))
                self.assertEqual(r.run(c, db, "d", bloom, "file", 0x8000), on)
            self.assertEqual(r.run(c, db, "d", "SELECT a FROM t1", "file", 0xffffffff), ("ok", [[i(1)]]))
            self.assertEqual(r.run(c, db, "d", bloom), on)
        finally:
            r.close()
            shutil.rmtree(d)


class ConnectionPragmas(unittest.TestCase):
    """A case's connection settings that the file does not keep are replayed
    before every statement: the last value of each wins, others are ignored."""

    def test_last_value_of_each(self):
        text = "Case {\n  PRAGMA automatic_index = 0;\n  PRAGMA cache_size = 5;\n  PRAGMA reverse_unordered_selects = 1;\n  PRAGMA automatic_index = 1\n}"
        self.assertEqual(P.connection_pragmas(text), b"PRAGMA automatic_index = 1;\nPRAGMA reverse_unordered_selects = 1;\n")
        self.assertEqual(P.connection_pragmas("CREATE TABLE t0(c0)"), b"")

    def test_replayed_before_each_statement(self):
        cli = os.environ.get("SQLITE3_CLI") or shutil.which("sqlite3")
        if not cli:
            self.skipTest("no sqlite3 CLI")
        r = P.Replayer(10)
        try:
            c = P.Cli("sys", cli, "x")
            self.assertEqual(r.run(c, None, "d", "PRAGMA reverse_unordered_selects", "memory"), ("ok", [[i(0)]]))
            r.prelude = P.connection_pragmas("PRAGMA reverse_unordered_selects = 1")
            for m in P.MODES:
                self.assertEqual(r.run(c, None, "d", "PRAGMA reverse_unordered_selects", m), ("ok", [[i(1)]]), m)
        finally:
            r.close()


class CaseReplay(unittest.TestCase):
    """after.db is main as committed, so a case that leaves a TEMP or
    ATTACHed database or an open transaction is replayed with its statements
    (run 20261021-0443's plans w5-3089, w6-4286 and w13-61234: "no such
    table: aux.t6", "no such column: c1", "no such table: temp.t1" on
    after.db alone, known:sqlite-wr-rowvalue-added-column once replayed)."""

    INPUT = ("Case {\n  file: SqliteFile(8192 bytes, digest 4794affa5d989ff4, page size 4096, 2 pages, 0 free)\n  CREATE TABLE t0(c0)\n"
             "  -- 3 statements\n  CREATE TABLE temp.t1(c0 ANY NOT NULL, PRIMARY KEY(c0)) WITHOUT ROWID, STRICT;\n"
             "  INSERT INTO t1 (c0) VALUES ('a\n}\nb');\n  ALTER TABLE t1 ADD COLUMN c1 INT;\n}\n[SELECT * FROM temp.t1 AS a0]\n")

    def test_case_script_from_input(self):
        script = P.case_script(self.INPUT)
        self.assertEqual(script, "  CREATE TABLE temp.t1(c0 ANY NOT NULL, PRIMARY KEY(c0)) WITHOUT ROWID, STRICT;\n"
                                 "  INSERT INTO t1 (c0) VALUES ('a\n}\nb');\n  ALTER TABLE t1 ADD COLUMN c1 INT;\n")
        self.assertEqual([w[:2] for _, _, w in P.split_statements(script)], [("CREATE", "TABLE"), ("INSERT", "INTO"), ("ALTER", "TABLE")])
        self.assertEqual(P.case_script("Case {\n  file: SqliteFile(0 bytes)\n  -- 0 statements\n}\n[]\n"), "")
        self.assertIsNone(P.case_script("CREATE TABLE t0(c0)"))

    def test_needs(self):
        n = P.case_needs
        self.assertEqual(n("CREATE TABLE t0(c0); INSERT INTO t0 VALUES (1);"), {"attach": False, "temp": False, "tail": None})
        self.assertTrue(n(P.case_script(self.INPUT))["temp"])
        for sql in ("CREATE TEMP TABLE t5(c0);", "CREATE TEMPORARY VIEW v AS SELECT 1;", "CREATE INDEX IF NOT EXISTS temp.i ON t1(c0);",
                    "CREATE TEMP TRIGGER w0 AFTER DELETE ON t5 BEGIN SELECT 1; END;"):
            self.assertTrue(n(sql)["temp"], sql)
        self.assertTrue(n("ATTACH DATABASE ':memory:' AS aux; CREATE TABLE aux.t6(c0);")["attach"])
        self.assertFalse(n("ATTACH DATABASE ':memory:' AS aux; DETACH aux;")["attach"])
        # w6-4286's shape: a transaction left open after a ROLLBACK TO.
        script = "SAVEPOINT s0;\nINSERT INTO t3 VALUES (1);\nRELEASE s0;\nBEGIN DEFERRED; SAVEPOINT s0;\nDELETE FROM t3;\nROLLBACK TO s0;\nALTER TABLE t5 ADD COLUMN c1 DEFAULT x'08';\n"
        tail = n(script)["tail"]
        self.assertEqual(script[tail:], "BEGIN DEFERRED; SAVEPOINT s0;\nDELETE FROM t3;\nROLLBACK TO s0;\nALTER TABLE t5 ADD COLUMN c1 DEFAULT x'08';\n")
        for closed in ("BEGIN; INSERT INTO t VALUES (1); COMMIT;", "BEGIN; END TRANSACTION;", "BEGIN; ROLLBACK;", "SAVEPOINT a; SAVEPOINT b; RELEASE SAVEPOINT a;",
                       "SAVEPOINT \"a\"; ROLLBACK TO a; RELEASE a;"):
            self.assertIsNone(n(closed)["tail"], closed)
        self.assertEqual(n("SAVEPOINT a; SAVEPOINT b; RELEASE b;")["tail"], 0)
        # A trigger's END (or a CASE's in it) is not a COMMIT.
        script = ("BEGIN; CREATE TRIGGER g AFTER INSERT ON t BEGIN SELECT CASE WHEN 1 THEN 2 END; INSERT INTO u VALUES (';'); END;"
                  " INSERT INTO t VALUES (1);")
        self.assertEqual(n(script)["tail"], 0)
        self.assertEqual([w[0] for _, _, w in P.split_statements(script)], ["BEGIN", "CREATE", "INSERT"])

    def test_schema_mismatch(self):
        self.assertTrue(P.schema_mismatch("no such table: aux.t6", "database disk image is malformed"))
        self.assertTrue(P.schema_mismatch("no such column: c1", ""))
        self.assertFalse(P.schema_mismatch("no such column: c1", "no such column: c1"))
        self.assertFalse(P.schema_mismatch("database disk image is malformed", "x"))

    def test_mismatched_replay_is_manual_not_new(self):
        """A replay on the wrong database ("no such table") is not the run's
        failure: manual, never new."""
        tri = Verdicts().triage({(c, "SELECT * FROM aux.t6"): ("err", "no such table: aux.t6") for c in ("trunk", "bug-a", "bug-b")})
        v = Verdicts().decide(tri, "the original query fails: SELECT * FROM aux.t6\n  -> database disk image is malformed")
        self.assertEqual(v["verdict"], "manual")
        self.assertIn("not the case's", v["reason"])
        msg = "results differ (tlp-where)\n  original: SELECT * FROM t5\n    -> Ok([[1]])\n  variant:  SELECT * FROM t5 WHERE c1\n    -> Ok([])"
        tri = Verdicts().triage({("trunk", "SELECT * FROM t5"): ("ok", [[i(1)]]), ("trunk", "SELECT * FROM t5 WHERE c1"): ("err", "no such column: c1")})
        self.assertEqual(Verdicts().decide(tri, msg)["verdict"], "manual")

    def test_which_replay(self):
        d = tempfile.mkdtemp(prefix="pretriage-test-")
        try:
            tri = Verdicts().triage({})
            stub = tri.r
            # Not needed: no case replay.
            self.assertIsNone(tri.case_replay(d, "Case {\n  -- 1 statements\n  CREATE TABLE t0(c0);\n}\n[]", {}))
            # An example from before before.db: reconstructed, main read-only for TEMP.
            stub.new_example()
            v = tri.case_replay(d, self.INPUT, {})
            self.assertEqual((v["how"], v["needs"], stub.case["readonly"], stub.case["before"]), ("reconstructed", ["TEMP"], True, None))
            self.assertEqual(stub.case["script"], P.case_script(self.INPUT).encode())
            # An open transaction alone: its statements on after.db, main writable.
            stub.new_example()
            v = tri.case_replay(d, "Case {\n  -- 3 statements\n  INSERT INTO t5 VALUES (1);\n  BEGIN;\n  ALTER TABLE t5 ADD COLUMN c1;\n}\n[]", {})
            self.assertEqual((v["how"], v["needs"], stub.case["readonly"]), ("reconstructed", ["open transaction"], False))
            self.assertEqual(stub.case["script"], b"BEGIN;\n  ALTER TABLE t5 ADD COLUMN c1;\n")
            # The runner's before.db and case.sql: faithful, with the WAL settings.
            with open(os.path.join(d, "before.db"), "wb") as f:
                f.write(b"")
            with open(os.path.join(d, "case.sql"), "w") as f:
                f.write("ATTACH ':memory:' AS aux;\nCREATE TABLE aux.t6(c0);\n")
            stub.new_example()
            v = tri.case_replay(d, self.INPUT, {"wal_settings": {"cache_size": 4}})
            self.assertEqual((v["how"], v["needs"], stub.case["before"], stub.case["wal"]), ("faithful", ["ATTACH"], os.path.join(d, "before.db"), {"cache_size": 4}))
        finally:
            shutil.rmtree(d)

    def test_real_cli(self):
        """On a real CLI: the case's statements before each statement, in
        every mode, their rows and errors not the statement's; from
        before.db faithfully, or on after.db reconstructed (main read-only
        for an ATTACHed database: a main write is not applied twice)."""
        cli = os.environ.get("SQLITE3_CLI") or shutil.which("sqlite3")
        if not cli:
            self.skipTest("no sqlite3 CLI")
        import sqlite3
        d = tempfile.mkdtemp(prefix="pretriage-test-")
        r = P.Replayer(10)
        c = P.Cli("sys", cli, "x")
        try:
            before, after = os.path.join(d, "before.db"), os.path.join(d, "after.db")
            for path, sql in ((before, "CREATE TABLE t0(c0); CREATE TABLE t5(c0 PRIMARY KEY) WITHOUT ROWID;"),
                              (after, "CREATE TABLE t0(c0); INSERT INTO t0 VALUES (1); CREATE TABLE t5(c0 PRIMARY KEY) WITHOUT ROWID; INSERT INTO t5 VALUES (1);")):
                con = sqlite3.connect(path)
                con.executescript(sql)
                con.close()
            # Faithful: a TEMP table, an ATTACHed one and an open transaction, from before.db.
            case = (b"INSERT INTO t5 VALUES (1);\nCREATE TEMP TABLE t1(c0);\nINSERT INTO t1 VALUES ('x');\nSELECT 42;\n"
                    b"ATTACH ':memory:' AS aux;\nCREATE TABLE aux.t6(c0);\nINSERT INTO aux.t6 VALUES (6);\n"
                    b"BEGIN; SAVEPOINT s0;\nINSERT INTO t5 VALUES (2);\nROLLBACK TO s0;\nALTER TABLE t5 ADD COLUMN c1 DEFAULT 8;\n")
            q = "SELECT (SELECT group_concat(c0 || ':' || c1) FROM t5), (SELECT c0 FROM temp.t1), (SELECT c0 FROM aux.t6)"
            r.set_case(case, before=before, wal={"cache_size": 4, "autocheckpoint": 1, "synchronous": "NORMAL"})
            for m in P.MODES:
                self.assertEqual(r.run(c, after, "d", q, m), ("ok", [[t("1:8"), t("x"), i(6)]]), m)
            self.assertEqual(r.run(c, after, "d", "SELECT * FROM nosuch", "file"), ("err", "no such table: nosuch"))
            # Reconstructed: main read-only, so after.db's rows stay as they are.
            r.new_example()
            r.set_case(b"INSERT INTO t0 VALUES (2);\nATTACH ':memory:' AS aux;\nCREATE TABLE aux.t6(c0);\nINSERT INTO aux.t6 SELECT c0 + 5 FROM t0;\n", readonly=True)
            for m in P.MODES:
                self.assertEqual(r.run(c, after, "d", "SELECT (SELECT group_concat(c0) FROM t0), (SELECT c0 FROM aux.t6)", m), ("ok", [[t("1"), i(6)]]), m)
            # Reconstructed open transaction: its statements on after.db.
            r.new_example()
            r.set_case(b"BEGIN;\nALTER TABLE t5 ADD COLUMN c1 DEFAULT 8;\n")
            for m in P.MODES:
                self.assertEqual(r.run(c, after, "d", "SELECT c0, c1 FROM t5", m), ("ok", [[i(1), i(8)]]), m)
            with open(after, "rb") as f:
                self.assertEqual(f.read(20)[18:20], b"\x01\x01")
        finally:
            r.close()
            shutil.rmtree(d)


class SkipListsMatchSqlancer(unittest.TestCase):
    """The skip lists are the current oracles' (ephyra-sqlite src/sqlancer.rs)."""

    PATH = os.environ.get("EPHYRA_SQLANCER") or os.path.join(P.ROOT_DIR, "case-studies", "sqlite", "generator", "src", "sqlancer.rs")

    def setUp(self):
        if not os.path.exists(self.PATH):
            self.skipTest(f"no {self.PATH}")
        with open(self.PATH, encoding="utf-8") as f:
            self.src = f.read()

    def body(self, name):
        start = self.src.index(f"fn {name}(")
        return self.src[start : self.src.index("\n}\n", start)]

    def strings(self, name):
        return re.findall(r'"((?:[^"\\]|\\.)*)"', self.body(name))

    def test_lists(self):
        self.assertEqual(tuple(self.strings("overflow")), P.OVERFLOW)
        self.assertEqual(tuple(self.strings("limit")), P.LIMITS)
        self.assertEqual(tuple(self.strings("json_error")), P.JSON_ERRORS)
        self.assertEqual(tuple(self.strings("io_error")), P.IO_ERRORS)

    def test_nondeterministic(self):
        """The statement error the oracles skip: the same date functions
        (expr.rs Func::DATE) and places."""
        body = self.body("nondeterministic")
        self.assertIn("crate::expr::Func::DATE", body)
        self.assertEqual(tuple(re.findall(r'"(an index|a CHECK constraint|a generated column)"', body)), P.PURE_PLACES)
        expr = open(os.path.join(os.path.dirname(self.PATH), "expr.rs"), encoding="utf-8").read()
        names = dict(re.findall(r"Func::(\w+) => \"(\w+)\",", expr))
        dates = re.search(r"pub const DATE: \[Func; \d+\] =\s*\[([^\]]*)\]", expr).group(1)
        self.assertEqual(tuple(names[f.strip().split("::")[1]] for f in dates.split(",") if f.strip()), P.DATE_FUNCS)
        self.assertTrue(P.nondeterministic("statement 7: UPDATE t0 SET c0 = 'now': non-deterministic use of julianday() in an index"))
        self.assertFalse(P.nondeterministic("non-deterministic use of random() in an index"))
        self.assertFalse(P.nondeterministic("non-deterministic functions prohibited in index expressions"))

    def test_skip_uses_these_three(self):
        self.assertEqual(re.findall(r"if (\w+)\(e\)", self.body("skip")), ["overflow", "limit", "json_error"])

    def test_known_issues(self):
        """pretriage mirrors the one per-query skip, invalid_text_pattern."""
        self.assertEqual(re.findall(r"(\w+)\(m, q\)", self.body("known_issue")), ["invalid_text_pattern"])

    def test_storage_compares_the_same_sql(self):
        """The storage oracle's variant is its original's SQL (on the WAL
        file), under the variant name `storage`."""
        body = self.body("storage")
        self.assertIn('agree(tally, "storage", mode(q, schema), &osql, &orig, &osql, &variant', body)

    def test_message_formats(self):
        """The formats parse_message reads are the ones sqlancer.rs writes."""
        for text in ('"{why} ({name})\\n  original: {osql}\\n    -> {orig:?}\\n  variant:  {vsql}\\n    -> {variant:?}"',
                     '"the original query fails: {sql}\\n  -> {e}"',
                     '"a SELECT of the compound fails: {}\\n  -> {e}"',
                     '"the window\'s input fails: {bsql}\\n  -> {e}"',
                     '"rows {k} and {} out of order (order-sorted)\\n  query: {ssql}\\n    -> {:?}\\n    -> {:?}"',
                     'stmts.join(";\\n             ")',
                     '"{fsql} (its rows {}.., {} of them)"',
                     '"{osql} (by definition, from its SELECTs\' rows)"',
                     '"{bsql} (the windows computed from its rows)"',
                     # plans-noopt's last line (_OPTS), and its variant names (noopt).
                     '"{m}\\n  optimizations off: {mask:#010x} ({name})"',
                     '"{message}\\n  optimizations off: {all:#010x} ({several}; none alone)"',
                     'concat!("plans-noopt-", stringify!($name))',
                     'agree(tally, "plans-noopt", mode, osql, orig, osql, &variant, nan)'):
            self.assertIn(text, self.src)


if __name__ == "__main__":
    unittest.main()
