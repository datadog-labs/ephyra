Standalone PoC. Needs a Python with `litellm` importable (e.g. `pip install litellm`, or
`PYTHONPATH=<litellm checkout>` plus its deps). No network: poc.py starts a local mock Gemini server.

    ./run.sh [python]     # prints both finish_reasons; exit 1 = bug shows, 0 = not, 2 = could not run

Checked 2026-10-27: exit 1 on main cb70bc56, on 52562c47 and on PyPI 1.106.5. Exit 0 with the loop's patch 001.
