**Description**

`DFlashAttention.__init__` in `torchspec/models/draft/dflash.py` (lines 191-195) always builds its own `DFlashRotaryEmbedding(head_dim, max_position_embeddings, base=rope_theta)`. It never reads `config.rope_scaling`.

`DFlashConfig` accepts `rope_scaling` and keeps it, so the saved draft config still declares it. With `rope_scaling={"rope_type": "linear", "factor": 2.0}` the draft trains with plain RoPE. Its cos/sin tables differ from the target model's. There is no error or warning. DSpark's draft model subclasses `DFlashDraftModel` and reuses its decoder layers, so it uses the same attention.

Expected: either the draft applies the scaling its config declares, or it refuses it. The project does both elsewhere. The `build_rotary_embedding` docstring in `torchspec/models/draft/llama3_eagle.py` says:

> Every draft that uses these rotary classes goes through here [...] Each attention block used to carry its own copy of this branch ladder, and the copies drifted [...] so drafts trained at 10000 while their config declared otherwise.

DFlash does not go through `build_rotary_embedding`. The GQA path in `dflash2.py` raises `ValueError("GQA DFlash2 training does not support rope_scaling")` instead of dropping it. transformers' `LlamaRotaryEmbedding` applies the linear factor for the same config.

**Steps to reproduce**

No tokenizer files or GPU are needed. Save as `poc.py` in a TorchSpec checkout:

```python
import torch
from transformers import LlamaConfig
from transformers.models.llama.modeling_llama import LlamaRotaryEmbedding
from torchspec.models.draft.dflash import DFlashAttention, DFlashConfig

scaling = {"rope_type": "linear", "factor": 2.0}
cfg = DFlashConfig(hidden_size=8, intermediate_size=8, num_attention_heads=1, num_key_value_heads=1,
                   head_dim=8, vocab_size=8, max_position_embeddings=8, rope_scaling=scaling)
ref = LlamaConfig(hidden_size=8, num_attention_heads=1, num_key_value_heads=1, head_dim=8,
                  max_position_embeddings=8, rope_scaling=scaling)

draft = DFlashAttention(cfg).rotary_emb
cos_hf, _ = LlamaRotaryEmbedding(ref)(torch.zeros(1, 8, 8), torch.arange(8)[None])

print("config rope_scaling:", cfg.rope_scaling)
print("torchspec    cos[pos 7]:", [round(x, 4) for x in draft.cos_cached[0, 0, 7, :4].tolist()])
print("transformers cos[pos 7]:", [round(x, 4) for x in cos_hf[0, 7, :4].tolist()])
```

```
python poc.py
# transformers: cos[pos 7] = [-0.9365, 0.9394, 0.9994, 1.0]   (position 7 / factor 2)
# torchspec:    cos[pos 7] = [0.7539, 0.7648, 0.9976, 1.0]    (position 7, no scaling)
```

**Observed output**

```
config rope_scaling: {'rope_type': 'linear', 'factor': 2.0, 'rope_theta': 10000.0}
torchspec    cos[pos 7]: [0.7539, 0.7648, 0.9976, 1.0]
transformers cos[pos 7]: [-0.9365, 0.9394, 0.9994, 1.0]
```

**Expected output**

```
torchspec    cos[pos 7]: [-0.9365, 0.9394, 0.9994, 1.0]
transformers cos[pos 7]: [-0.9365, 0.9394, 0.9994, 1.0]
```

(or a clear error that DFlash does not support `rope_scaling`).

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux x86_64
- CPU only, no GPU

**Latest release**

The repository has no tagged releases. `DFlashAttention` has built `DFlashRotaryEmbedding` without reading `rope_scaling` since DFlash was added in #64 (760a552), so every version with DFlash has it.

**Related issues and PRs**

- #208 (open): `build_rotary_embedding` ignores `rope_theta` for linear, dynamic and mrope. Different cause; DFlash never calls `build_rotary_embedding`, so fixing #208 does not fix this.
- #207 (open): YaRN `rope_scaling` without beta_fast/beta_slow crashes `build_rotary_embedding`. Different code path.
- #165 and #166 consolidated the rotary branch ladders and `rope_theta` resolution. DFlash was not moved onto them.
- A YaRN `rope_scaling` cannot currently be passed to `DFlashConfig` on transformers 5.x at all (it raises AttributeError in `__init__`). That is a separate problem; this report is about the scaling types that do construct, such as linear and dynamic.

A fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
