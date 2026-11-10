**Description**

`K3Renderer.get_assistant_token_ids()` in `torchspec/data/renderers/kimi_k3.py` returns a fixed header for the training-side matcher:

```python
return [*self.assistant_header_ids, self.sep_id], [self.eom_id], 0
```

The tokens are `<|open|>message role="assistant"<|sep|>`. The Kimi-K3 checkpoint encoder (`encoding_k3.py`) adds a `name` attribute to the header when the message has one, which gives `<|open|>message role="assistant" name="x"<|sep|>`. The fixed header does not match it. `compute_assistant_loss_mask` therefore finds no assistant turn and gives the named turn a loss mask of all zeros. There is no error or warning.

The renderer's own mask handles this case. `K3Renderer._assistant_spans` matches the role prefix and then scans forward to `<|sep|>`, so it supervises the named turn.

Expected: the matcher mask equals the renderer mask. The method's docstring says it is the "K3-aware matcher contract for post-vLLM token IDs". `tests/test_kimi_k3_renderer.py::test_training_matcher_reproduces_k3_mask_after_media_expansion` asserts `dynamic_mask == renderer.compute_loss_mask(...)`. That test file's `StubK3Tokenizer` also models named assistant headers (`if name := message.get("name")`), and `test_last_turn_only_keeps_only_the_last_semantic_assistant_span` shows that the renderer supervises a named turn.

Where this affects training: with the legacy `kimi-k3` renderer (`dataset.rendering_backend: huggingface`), `train_config.py` sets `dynamic_loss_mask=True`. Text-only samples keep the renderer's `packed_loss_mask`. Multimodal samples have that mask dropped (`_drop_stale_multimodal_masks`) and recomputed with this matcher after media expansion. So named assistant turns in multimodal K3 samples are not trained. I worked this path out from the code. I did not run a full training job. The SMG route (`smg-kimi-k3`) does not use the matcher.

**Steps to reproduce**

You need a TorchSpec checkout on the Python path, CPU only. The script downloads the Kimi-K3 tokenizer files at a pinned revision. `trust_remote_code` is needed for `tokenization_kimi.py`. No weights are downloaded. Save it as `poc.py`:

```python
import sys

import torch
from huggingface_hub import snapshot_download
from transformers import AutoTokenizer
from torchspec.data.renderers.kimi_k3 import K3Renderer
from torchspec.models.ops.loss_mask import compute_assistant_loss_mask

path = snapshot_download(
    "moonshotai/Kimi-K3",
    revision="9f62e4e9fffbd0a83ddd60e1c209d828994b3569",
    allow_patterns=["config.json", "configuration_kimi_k3.py", "encoding_k3.py", "generation_config.json",
                    "tiktoken.model", "tokenization_kimi.py", "tokenizer_config.json"],
)
tok = AutoTokenizer.from_pretrained(path, trust_remote_code=True)
renderer = K3Renderer(tok)

messages = [
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": "hello", "name": "x"},
]
ids, renderer_mask = renderer.render(messages, max_seq_length=4096)
header, end, skip = renderer.get_assistant_token_ids()
matcher_mask = compute_assistant_loss_mask(torch.tensor(ids), header, end, skip_after_header=skip).tolist()

supervised = lambda mask: tok.decode([t for t, m in zip(ids, mask) if m])
print("rendered:      ", repr(tok.decode(ids)))
print("renderer mask: ", repr(supervised(renderer_mask)))
print("matcher mask:  ", repr(supervised(matcher_mask)))
sys.exit(0 if matcher_mask == renderer_mask else 1)
```

```
python poc.py
# K3Renderer.render mask (expected): supervises the assistant body "...hello..."
# get_assistant_token_ids + compute_assistant_loss_mask (torchspec): supervises nothing
```

**Observed output** (the long thinking-effort system prompt is shortened to `...`)

```
rendered:       '<|open|>message role="system" type="thinking-effort"<|sep|>...<|close|>message<|sep|><|end_of_msg|><|open|>message role="user"<|sep|>hi<|close|>message<|sep|><|end_of_msg|><|open|>message role="assistant" name="x"<|sep|><|open|>think<|sep|><|close|>think<|sep|><|open|>response<|sep|>hello<|close|>response<|sep|><|close|>message<|sep|><|end_of_msg|>'
renderer mask:  '<|open|>think<|sep|><|close|>think<|sep|><|open|>response<|sep|>hello<|close|>response<|sep|><|close|>message<|sep|>'
matcher mask:   ''
```

**Expected output**

```
renderer mask:  '<|open|>think<|sep|><|close|>think<|sep|><|open|>response<|sep|>hello<|close|>response<|sep|><|close|>message<|sep|>'
matcher mask:   '<|open|>think<|sep|><|close|>think<|sep|><|open|>response<|sep|>hello<|close|>response<|sep|><|close|>message<|sep|>'
```

If you remove `"name": "x"`, both masks come out as the expected output above. So only the named header fails to match.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main, re-checked 2026-11-02)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.10, Linux x86_64 (Ubuntu, glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged releases. `get_assistant_token_ids` has returned this fixed header since the K3 renderer was added in #164 (commit dd5f09a, 2026-08-09). Every version of main since then has the bug.

**Related issues and PRs**

- #164 "feat(dataset): Kimi-K3 conversation renderer" added the method.
- #206 "compute_assistant_loss_mask(last_turn_only=True) masks an earlier assistant turn when the last one is empty" is about the same matcher function, but it is a different bug.
- I found no issue or PR about named assistant turns or the K3 matcher header. I searched the titles of all open and closed issues and PRs.

A fix with a test is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
