**Description**

`build_rotary_embedding` (`torchspec/models/draft/llama3_eagle.py:628-699`) reads `rope_theta` from the config and passes it as `base=` to the default, llama3 and yarn classes. The other three branches build their class without `base=`, so it falls back to the class default `base=10000`:

- `linear`: `LlamaLinearScalingRotaryEmbedding(dim, max_position_embeddings=..., scaling_factor=...)` (line 662)
- `dynamic`: `LlamaDynamicNTKScalingRotaryEmbedding(dim, max_position_embeddings=..., scaling_factor=...)` (line 670)
- `mrope`: `LlamaMutiRotaryEmbedding(dim, max_position_embeddings=...)` (line 686)

A draft whose config declares, for example, `rope_theta: 1000000` with one of these scaling types trains with RoPE at base 10000. Its cos/sin tables differ from the target model's, and nothing reports it. Auto-generated draft configs copy both `rope_theta` and `rope_scaling` from the target model (`torchspec/config/utils.py`), so any target with one of these scaling types and a non-10000 theta is affected.

The dynamic branch has a second problem. `LlamaRotaryEmbedding.__init__` pre-builds the cache with `seq_len=max_position_embeddings + 20` (lines 393-397). `LlamaDynamicNTKScalingRotaryEmbedding._set_cos_sin_cache` sees `seq_len > max_position_embeddings` and NTK-scales the base. So the cached frequencies are scaled even for sequences shorter than the context, at any `rope_theta`, including 10000.

Expected behaviour:

- The `build_rotary_embedding` docstring: "Each attention block used to carry its own copy of this branch ladder, and the copies drifted: the MLA one dropped `base` on the YaRN branch, so drafts trained at 10000 while their config declared otherwise." The linear, dynamic and mrope branches still drop it.
- `tests/test_draft_rope.py`: "Tests that a draft's rotary base is the one its config declares."
- transformers' `LlamaRotaryEmbedding` uses the config's `rope_theta` for `linear` and `dynamic`. Its dynamic NTK docs (`_compute_dynamic_ntk_parameters` in `modeling_rope_utils.py`) say: "If seq_len <= max_position_embeddings, this factor has no effect."

**Steps to reproduce**

CPU only, no downloads. Save as `poc.py` and run it from a TorchSpec checkout:

```python
import torch
from transformers import LlamaConfig
from transformers.models.llama.modeling_llama import LlamaRotaryEmbedding
from torchspec.models.draft.llama3_eagle import build_rotary_embedding

THETA = 1e6
for rope_type in ["linear", "dynamic", "mrope"]:
    scaling = {"rope_type": rope_type}
    if rope_type != "mrope":
        scaling["factor"] = 2.0
    cfg = LlamaConfig(hidden_size=8, num_attention_heads=1, num_key_value_heads=1, head_dim=8,
                      max_position_embeddings=64, rope_theta=THETA, rope_scaling=scaling)
    draft = build_rotary_embedding(cfg, dim=8, max_position_embeddings=64)
    line = f"{rope_type:8} torchspec base={draft.base}"
    if rope_type != "mrope":  # transformers has no 'mrope' rope_type to compare against
        cos_hf, _ = LlamaRotaryEmbedding(cfg)(torch.zeros(1, 64, 8), torch.arange(64)[None])
        diff = (draft.cos_cached[0, 0, :64].float() - cos_hf[0]).abs().max().item()
        line += f"  max |cos - transformers cos| over positions 0..63: {diff:.4f}"
    print(line)
```

```
python poc.py
```

**Observed and expected output**

Actual output on c4fc3e0:

```
linear   torchspec base=10000  max |cos - transformers cos| over positions 0..63: 1.5791
dynamic  torchspec base=10000  max |cos - transformers cos| over positions 0..63: 1.4477
mrope    torchspec base=10000
```

Expected output:

```
linear   torchspec base=1000000.0  max |cos - transformers cos| over positions 0..63: 0.0000
dynamic  torchspec base=1000000.0  max |cos - transformers cos| over positions 0..63: 0.0000
mrope    torchspec base=1000000.0
```

With `THETA = 10000.0`, linear and mrope agree with transformers (which is why the existing tests do not catch them), but dynamic still differs because of the cache pre-build:

```
linear   torchspec base=10000  max |cos - transformers cos| over positions 0..63: 0.0000
dynamic  torchspec base=10000  max |cos - transformers cos| over positions 0..63: 0.7573
mrope    torchspec base=10000
```

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main)
- torch 2.15.0, transformers 5.22.1 (also seen with 5.20.2)
- Python 3.12, Linux x86_64 (Ubuntu 24.04)
- CPU only, no GPU

**Latest release**

The repository has no tagged releases. These branches were built without `base` already in the initial commit (4643a09), and the `+ 20` cache pre-build is from the same commit. #165 (3097987) moved the branches into `build_rotary_embedding` and fixed the same omission on the YaRN branch, but not on these three.

**Related issues and PRs**

- #166 "fix: resolve rope_theta from wherever the config stores it" fixed where `rope_theta` is read from. That part works here; the value is just not passed on.
- #165 consolidated the branch ladders and fixed the YaRN `base`.
- #101 and #103 are other RoPE config fixes. They do not cover this case.
- #207 is a separate YaRN crash in the same function.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
