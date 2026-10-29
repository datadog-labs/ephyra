`python3 poc.py` with litellm installed (`pip install litellm`) or `PYTHONPATH=<litellm checkout>`.
It starts a local mock OpenAI server, so no network or key is needed. Exit 1 = bug reproduces, 0 = fixed.
`./repro.sh <harness-build-dir>` runs it with the harness build's venv and source tree.
