"""torchspec_smg.Tokenizer.render rounds integers outside the i64/u64 range to floats.

Usage: python poc.py [tokenizer-dir]
Without an argument, downloads the tokenizer files of Qwen/Qwen3-0.6B at the revision
pinned in tests/tokenizer_contract/fixtures/manifest.json. CPU only.
Exit 1 if SMG silently changes a number, 0 if not.
"""
import json
import sys

from transformers import AutoTokenizer
from torchspec_smg import Tokenizer

if len(sys.argv) > 1:
    path = sys.argv[1]
else:
    from huggingface_hub import snapshot_download

    path = snapshot_download(
        "Qwen/Qwen3-0.6B",
        revision="c1899de289a04d12100db370d81485cdf75e47ca",
        allow_patterns=["config.json", "tokenizer.json", "tokenizer_config.json", "vocab.json", "merges.txt"],
    )

smg = Tokenizer(path)
hf = AutoTokenizer.from_pretrained(path)


def arguments(text):
    return text.split('"arguments": ')[1].split("}")[0] + "}"


bad = False
for n in [2**64 - 1, 10**20 + 1, -(2**63) - 1]:
    messages = [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "", "tool_calls": [
            {"type": "function", "function": {"name": "f", "arguments": {"x": n}}}]},
    ]
    print(f"hf:  {arguments(hf.apply_chat_template(messages, tokenize=False))}")
    try:
        smg_args = arguments(smg.render(messages)["text"])
    except ValueError as exc:  # an explicit error is acceptable, a silent change is not
        print(f"smg: ValueError: {exc}")
        continue
    same = json.loads(smg_args)["x"] == n
    bad |= not same
    print(f"smg: {smg_args}  {'ok' if same else 'WRONG VALUE'}")
sys.exit(1 if bad else 0)
