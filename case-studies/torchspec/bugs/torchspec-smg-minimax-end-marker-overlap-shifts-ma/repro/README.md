Run against a TorchSpec install that has the SMG binding (`pip install -e .` from the repo root builds it):

    python poc.py                 # downloads MiniMaxAI/MiniMax-M2.5 tokenizer files at 2515a3ea
    python poc.py <tokenizer-dir> # or use local files (chat_template.jinja, tokenizer.json, ...)

Exit 1: SmgRenderer supervised the wrong text (the bug).
Exit 0: every row was supervised correctly or rejected with ValueError.
