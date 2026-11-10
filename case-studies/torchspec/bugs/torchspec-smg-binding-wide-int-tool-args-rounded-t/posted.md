**Description**

`torchspec_smg.Tokenizer.render` changes integers below -2^63 or above 2^64-1 into rounded floats. The integer can be in tool-call arguments or anywhere else in a message. `100000000000000000001` is rendered as `1e+20`, and `-9223372036854775809` as `-9.223372036854776e+18`. No error or warning is raised. `SmgRenderer.render` uses this binding, so the training `input_ids` carry the wrong number too. On the example below they decode to `"x": 1e+20`. The HF chat template renders the exact integer.

`docs/smg-tokenization.md` says:

> SMG owns tool argument serialization, including nested objects/arrays and floats. Binding >=0.1.3 enables exact JSON float round-tripping; decimal spelling may change, but the parsed numeric value must not.

and

> Missing bindings, unsupported media, and rendering errors are actionable failures, never silent backend swaps.

The parsed value changes here (10^20+1 becomes 10^20), and nothing reports it. Expected: the exact integer, or a clear error if the binding cannot represent it.

Likely cause: `render()` sends the request through `json.dumps`, and `lib.rs` parses it with `serde_json::from_str` into `serde_json::Value`. The crate is built with `preserve_order` and `float_roundtrip` but without `arbitrary_precision`, so an integer that fits neither i64 nor u64 becomes an f64.

The SMG backend is the default (`rendering_backend: auto`) for the qwen3-thinking, qwen, llama3 and minimax-m2 templates. The same rounding shows up with the minimax-m2 template.

**Steps to reproduce**

`poc.py` (CPU only; downloads only the tokenizer files of `Qwen/Qwen3-0.6B` at the revision pinned in `tests/tokenizer_contract/fixtures/manifest.json`):

```python
import json
import sys

from huggingface_hub import snapshot_download
from transformers import AutoTokenizer
from torchspec_smg import Tokenizer

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
    smg_args = arguments(smg.render(messages)["text"])
    same = json.loads(smg_args)["x"] == n
    bad |= not same
    print(f"smg: {smg_args}  {'ok' if same else 'WRONG VALUE'}")
sys.exit(1 if bad else 0)
```

Command:

```
python poc.py
```

**Observed and expected output**

```
python poc.py
hf:  {"x": 18446744073709551615}
smg: {"x": 18446744073709551615}  ok
hf:  {"x": 100000000000000000001}
smg: {"x": 1e+20}  WRONG VALUE
hf:  {"x": -9223372036854775809}
smg: {"x": -9.223372036854776e+18}  WRONG VALUE
# transformers apply_chat_template (hf lines): the exact integer every time
# torchspec_smg.Tokenizer.render (smg lines): 10^20+1 and -2^63-1 come back as rounded floats
```

The script exits with status 1. Values from -2^63 to 2^64-1 are rendered exactly.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (main, 2026-10-29), `torchspec_smg` 0.1.3, default build features
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3, huggingface_hub 2.4.0
- Python 3.12.3, Linux 6.8 x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged release. The binding was added in #201 (commit 4094382), and every commit since then has the bug, including current main.

**Related issues and PRs**

I searched open and closed issues and PRs for SMG, integer, float, precision, JSON, tool arguments and serde. Nothing matches. Related: #201, which added the SMG binding.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
