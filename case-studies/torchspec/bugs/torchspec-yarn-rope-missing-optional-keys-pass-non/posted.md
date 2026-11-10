**Description**

When a draft config's `rope_scaling` has `rope_type` (or `type`) `"yarn"` but leaves out the optional `beta_fast`, `beta_slow`, or both `mscale` keys, building the draft model raises `TypeError: unsupported operand type(s) for *: 'NoneType' and 'int'`.

`build_rotary_embedding` (`torchspec/models/draft/llama3_eagle.py:687-698`) passes `rget("beta_fast")`, `rget("beta_slow")`, `rget("mscale")`, `rget("mscale_all_dim")` and `rget("original_max_position_embeddings")` to `LlamaYarnRotaryEmbedding`. Each of these is `None` when the key is absent. The `None` overrides the class defaults (`beta_fast=32, beta_slow=1, mscale=1, mscale_all_dim=0`, llama3_eagle.py:558-562). The crash happens in `yarn_find_correction_dim` or `yarn_get_mscale`.

Expected: these keys are optional. The rope parameters docstring in transformers (`modeling_rope_utils.py`), which TorchSpec reads through `LlamaConfig`, says:

> `beta_fast` (`float`, *optional*, defaults to 32) ... `beta_slow` (`float`, *optional*, defaults to 1)

TorchSpec also agrees with these defaults:
- `_normalize_rope_scaling` in `torchspec/config/utils.py`, added in #54, fills `beta_fast=32.0, beta_slow=1.0, mscale=1.0, mscale_all_dim=0.0` for yarn.
- `tests/test_draft_model_config.py::test_generate_draft_model_config_fills_yarn_defaults` checks this.

That normalization runs only in `generate_draft_model_config`, where the draft config is generated from the target model. It does not run for a draft config file passed with `draft_model_config` (`AutoDraftModelConfig.from_file`, `train_entry.py:250`), or for a config built in code. Those still crash. A plain Hugging Face style block such as `{"rope_type": "yarn", "factor": 4.0, "original_max_position_embeddings": 32768}` is enough to trigger it. The shipped `configs/draft_models/*.json` set every YaRN key, so they do not hit it.

**Steps to reproduce**

CPU only. No tokenizer files or downloads are needed.

```python
# poc.py
from transformers import LlamaConfig
from transformers.models.llama.modeling_llama import LlamaRotaryEmbedding as HFRope
from torchspec.models.draft.llama3_eagle import LlamaForCausalLMEagle3

cfg = LlamaConfig(
    hidden_size=64, num_attention_heads=4, num_key_value_heads=2, intermediate_size=128,
    num_hidden_layers=1, vocab_size=256, draft_vocab_size=256, max_position_embeddings=256,
    rope_scaling={"rope_type": "yarn", "factor": 4.0, "original_max_position_embeddings": 64},
)
HFRope(cfg)
print("transformers: ok")
LlamaForCausalLMEagle3(cfg, attention_backend="sdpa")
print("torchspec: ok")
```

```
python poc.py
# transformers: ok
# torchspec: TypeError: unsupported operand type(s) for *: 'NoneType' and 'int'
```

Expected output:

```
transformers: ok
torchspec: ok
```

Actual output (paths shortened):

```
transformers: ok
Traceback (most recent call last):
  File "poc.py", line 12, in <module>
    LlamaForCausalLMEagle3(cfg, attention_backend="sdpa")
  File "torchspec/models/draft/llama3_eagle.py", line 2483, in __init__
    self.midlayer = LlamaDecoderLayer(config, attention_backend=attention_backend)
  File "torchspec/models/draft/llama3_eagle.py", line 2416, in __init__
    self.self_attn = LlamaAttention(config=config)
  File "torchspec/models/draft/llama3_eagle.py", line 1248, in __init__
    self._init_rope()
  File "torchspec/models/draft/llama3_eagle.py", line 1251, in _init_rope
    self.rotary_emb = build_rotary_embedding(
  File "torchspec/models/draft/llama3_eagle.py", line 688, in build_rotary_embedding
    return LlamaYarnRotaryEmbedding(
  File "torchspec/models/draft/llama3_eagle.py", line 570, in __init__
    super().__init__(dim, max_position_embeddings, base, device)
  File "torchspec/models/draft/llama3_eagle.py", line 393, in __init__
    self._set_cos_sin_cache(
  File "torchspec/models/draft/llama3_eagle.py", line 584, in _set_cos_sin_cache
    low, high = yarn_find_correction_range(
  File "torchspec/models/draft/llama3_eagle.py", line 531, in yarn_find_correction_range
    low = math.floor(yarn_find_correction_dim(low_rot, dim, base, max_position_embeddings))
  File "torchspec/models/draft/llama3_eagle.py", line 524, in yarn_find_correction_dim
    return (dim * math.log(max_position_embeddings / (num_rotations * 2 * math.pi))) / (
TypeError: unsupported operand type(s) for *: 'NoneType' and 'int'
```

The same error happens when that `rope_scaling` is written into a draft config JSON (for example a copy of `configs/draft_models/qwen3_8b_eagle3.json`) and loaded with `AutoDraftModelConfig.from_file`. If only `beta_slow` is missing, the error is `unsupported operand type(s) for *: 'float' and 'NoneType'`.

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux x86_64 (glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no tagged releases. The `None` pass-through has been there since the initial commit (4643a09). It moved into `build_rotary_embedding` in #165 (3097987).

**Related issues and PRs**

- #54 "Fix kimi yarn settings for draft model" added the YaRN defaults, but only for auto-generated draft configs.
- #166, #103 and #101 are other RoPE config fixes. They do not cover this case.

A fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
