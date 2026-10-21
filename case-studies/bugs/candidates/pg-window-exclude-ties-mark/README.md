# Candidate: window frame "N FOLLOWING .. EXCLUDE TIES" raises "cannot fetch row before WindowObject's mark position" (XX000)

Found 2026-10-14 by the workspace fuzz runner (plans oracle, the original query fails). repro.sql: four bool rows,
`first_value(c1) OVER (ORDER BY c1 ROWS BETWEEN 2 FOLLOWING AND UNBOUNDED FOLLOWING EXCLUDE TIES)` -> ERROR XX000 on
master 158b6d5109 and 18.6 (both rechecked in the main session). Not reported. Triage pending.

## Triage (2026-10-14): known upstream, do not file

Duplicate of BUG #19732 (pgsql-bugs, reported 2026-09-30 against 18.6; #19731 closed as its duplicate). Our exact
XX000 query was posted to that thread on 2026-10-05; Andrey Rachitskiy's patch v4 (2026-10-12, Message-ID
CAB8bMitM01bphvvik4hPtMh93=FMLUaFcKP+Jpch3mzHwZrYUw@mail.gmail.com) fixes it, two reviewers approve, not committed
(origin/master aadce2c10ff still reproduces). Since PostgreSQL 11 (0a459cec96d). ROWS frames that exclude the
current row with EXCLUDE TIES: first_value/last_value/nth_value give NULL or a value shifted by one row; with an
offset of 2 FOLLOWING or more, the mark-position error. Matrix, minimal.sql, upstream-v4-0001.patch in this folder.
Silent wrong results rechecked in the main session on master: `(1),(1),(2)` with ROWS 1 FOLLOWING .. UNBOUNDED
FOLLOWING EXCLUDE TIES gives NULL for the first row (expected 2).
