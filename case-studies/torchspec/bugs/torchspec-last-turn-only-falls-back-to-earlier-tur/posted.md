**Description**

`torchspec.models.ops.loss_mask.compute_assistant_loss_mask` with `last_turn_only=True` does not keep the last assistant turn. It keeps the last contiguous run of 1s in the mask. If the final assistant turn has empty content, it adds no 1s. The previous assistant turn then survives and is marked for loss.

The docstring says:

> Compute loss mask where 1s mark assistant content tokens only.

and

> last_turn_only: If True, only the last assistant turn is marked.

Expected: only the last assistant turn is marked. When that turn is empty, the mask is all zeros. Actual: an earlier turn's tokens are marked.

This matters in training with `dynamic_loss_mask`. `resolve_loss_mask` in `torchspec/data/utils.py` calls this function with the per-sample `last_turn_loss_only` (set from `has_thinking`) or the global setting. An all-zero mask makes it return `None`. Instead, a conversation whose final assistant message is empty trains on an earlier assistant answer, even though last-turn-only loss was requested.

The post-pass came with #37 (commit 40a4e8d, "[Bug Fix] Support last_turn_loss_only in dynamic_loss_mask path"). It finds the last 1 and walks back to the start of its run. A possible fix is to have the scan return where the last matched header's content starts, and zero everything before that.

**Steps to reproduce**

`poc.py` (CPU only; downloads only the tokenizer files of `Qwen/Qwen3-0.6B` at the revision pinned in `tests/tokenizer_contract/fixtures/manifest.json`). The header, end and skip values match what `get_assistant_token_ids` derives for the qwen templates:

```python
import sys

from huggingface_hub import snapshot_download
from transformers import AutoTokenizer

from torchspec.models.ops.loss_mask import compute_assistant_loss_mask

path = snapshot_download(
    "Qwen/Qwen3-0.6B",
    revision="c1899de289a04d12100db370d81485cdf75e47ca",
    allow_patterns=["config.json", "tokenizer.json", "tokenizer_config.json", "vocab.json", "merges.txt"],
)
tok = AutoTokenizer.from_pretrained(path)

# Same matcher as torchspec.utils.processing.get_assistant_token_ids for qwen templates.
header = tok.encode("<|im_start|>assistant", add_special_tokens=False)
end = tok.encode("<|im_end|>", add_special_tokens=False)

text = (
    "<|im_start|>user\nhi<|im_end|>\n"
    "<|im_start|>assistant\nOLD ANSWER<|im_end|>\n"
    "<|im_start|>user\nagain<|im_end|>\n"
    "<|im_start|>assistant\n<|im_end|>\n"  # last assistant turn is empty
)
ids = tok.encode(text, add_special_tokens=False)
mask = compute_assistant_loss_mask(ids, header, end, last_turn_only=True, skip_after_header=1)
masked = tok.decode([t for t, m in zip(ids, mask.tolist()) if m])
print(f"last_turn_only=True trains on: {masked!r}")
sys.exit(1 if masked else 0)
```

Command:

```
python poc.py
```

The same thing as a one-liner without a tokenizer (header `[1]`, end `[4]`, two turns, the last one empty):

```
python -c "from torchspec.models.ops.loss_mask import compute_assistant_loss_mask as f; print(f([1,2,4,1,4],[1],[4],last_turn_only=True).tolist())"
```

**Observed and expected output**

```
python poc.py
last_turn_only=True trains on: 'OLD ANSWER'
# docstring ("only the last assistant turn is marked"): trains on: ''
# torchspec main: trains on: 'OLD ANSWER' (the earlier turn), exit status 1
```

```
python -c "from torchspec.models.ops.loss_mask import compute_assistant_loss_mask as f; print(f([1,2,4,1,4],[1],[4],last_turn_only=True).tolist())"
[0, 1, 0, 0, 0]
# docstring: [0, 0, 0, 0, 0]
# torchspec main: [0, 1, 0, 0, 0]
```

When the last assistant turn has content, the result is correct: `f([1,2,4,1,3,4],[1],[4],last_turn_only=True)` gives `[0, 0, 0, 0, 1, 0]`.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (main, 2026-10-29)
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3, huggingface_hub 2.4.0, numba 0.68.1, numpy 2.3.5
- Python 3.12.3, Linux 6.8 x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged release. The code came in #37 (commit 40a4e8d). Every commit since then has the bug, including current main.

**Related issues and PRs**

I searched open and closed issues and PRs for last_turn, last turn, loss_mask, mask, empty, dynamic, multi-turn and thinking. Nothing reports this bug. Related: #37, which added `last_turn_only` support to the dynamic loss mask path.

A fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
