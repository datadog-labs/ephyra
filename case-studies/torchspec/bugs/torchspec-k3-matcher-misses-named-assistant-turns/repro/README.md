# PoC: K3 training matcher misses named assistant turns

Requirements: a TorchSpec checkout installed or on the Python path, CPU only. The script downloads
the Kimi-K3 tokenizer files from moonshotai/Kimi-K3 at revision
9f62e4e9fffbd0a83ddd60e1c209d828994b3569. It uses `trust_remote_code`; no model weights are needed.

    python poc.py

Exit code 1 means the bug reproduces: the matcher mask differs from the renderer mask. Exit code 0 means it does not.

Results:
- TorchSpec c4fc3e0 (upstream main, 2026-11-02): exit 1, and the matcher mask is empty.
- The same commit with the candidate fix: exit 0.
- If you delete `"name": "x"`, both masks agree on c4fc3e0. This is the control.
