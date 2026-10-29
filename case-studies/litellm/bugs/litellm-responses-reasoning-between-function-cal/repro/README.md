# repro
- `minimal.py`: about 15 lines, offline. Exit 10 = bug, 20 = request was built (fixed).
- `poc.py`: end to end. A mock Gemini server on 127.0.0.1 streams [functionCall, thought, functionCall];
  the output is replayed with both tool results. Exit 10 = bug, 20 = fixed.
- `repro.sh <build-dir>`: runs poc.py with `$LITELLM_PY` from `<build-dir>/env.sh`; exit 1 = reproduces, 0 = not.
  Simplest use: `python poc.py` in any environment with litellm installed.
