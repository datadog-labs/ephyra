Run against a TorchSpec install that has the SMG binding (`pip install -e .` from the repo root builds it):

    python poc.py                 # downloads Qwen/Qwen3-0.6B tokenizer files at c1899de2
    python poc.py <tokenizer-dir> # or use local files (tokenizer.json, tokenizer_config.json, ...)

Exit 1: SMG silently changed a number (the bug). Exit 0: every number was kept or rejected with ValueError.
