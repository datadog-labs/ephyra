# repro

`poc.py` starts a local mock Gemini server, calls `litellm.completion` with a `gemini/` model and a history
holding two identical parallel tool calls (`get_time({})`, ids call_1 and call_2) plus their two tool results,
and counts the functionCall and functionResponse parts in the request litellm sends. Exit 1 when the counts differ (bug), 0 when they match.

Run: `python poc.py` with litellm installed, or `./repro.sh <build-dir>` with a loop build (sources `<build-dir>/env.sh`).

Checked 2026-10-28: exit 1 on main c7c895db, on 52562c47 and on PyPI 1.106.5. Exit 0 with the loop's patch 006.
