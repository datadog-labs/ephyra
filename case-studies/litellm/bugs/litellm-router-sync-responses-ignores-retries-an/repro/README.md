Run: `./repro.sh /path/to/python` (a Python where `import litellm` works; for a source checkout set PYTHONPATH to it).
Needs no network: poc.py starts its own mock OpenAI server on 127.0.0.1 (first POST 503, then 200).
Exit 1 = bug reproduces (sync responses() makes 1 request and raises), 0 = fixed (it retries), 2 = cannot decide.
Also `fallbacks.py`: same idea for `fallbacks=[{"primary": ["backup"]}]`; prints which deployments got requests (run it with the same python, LITELLM_LOCAL_MODEL_COST_MAP=True NO_PROXY="*").
