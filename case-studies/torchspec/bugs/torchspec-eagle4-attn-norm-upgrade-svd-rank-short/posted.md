**Description**

`load_initial_draft_weights` can warm-start an EAGLE-4 draft with `attn_norm: true` from an EAGLE3 checkpoint. `_upgrade_to_eagle4` in `torchspec/training/checkpoint.py` does this by SVD-factoring the stacked q/k/v. The comment there says:

> attn_norm: SVD-factor the stacked EAGLE3 q/k/v into eh_proj (2H -> H) and H-wide q/k/v.

It calls `torch.linalg.svd(stacked, False)` (reduced SVD). That gives only `min(rows, 2H)` singular directions, where `rows = (num_attention_heads + 2 * num_key_value_heads) * head_dim`. When `rows < hidden_size`, `vh[:hidden_size]` has only `rows` rows, and `u[:, :hidden_size]` only `rows` columns. So `eh_proj` comes out `rows x 2H` instead of `H x 2H`, and q/k/v come out `rows` wide instead of `H` wide. `_load_initial_weights` then rejects the upgrade's own output with `shape_mismatch`.

Expected: the warm start loads, with an `H x 2H` eh_proj and H-wide q/k/v. The test `test_upgrades_a_pruned_eagle3_draft_to_eagle4` in `tests/test_initial_draft_model_path.py` says:

> q/k/v together are no wider than hidden_size, so the eh_proj factoring is exact.

That test uses rows equal to hidden_size (64). With rows below hidden_size (32 in the script below) the factoring should still be exact, but the load fails instead.

The case needs an explicit `head_dim` with `num_attention_heads * head_dim < hidden_size`. With the default `head_dim = hidden_size / num_attention_heads`, rows is always at least hidden_size.

In practice this is low priority: none of the shipped `configs/draft_models/*.json` meets the condition (the tightest, `qwen3_8_27b_eagle4.json`, has 8192 stacked rows for hidden_size 5120), and common target models have `num_attention_heads * head_dim >= hidden_size`. It only shows up with small or unusual configs. The fix is small and keeps the factorization exact, so the comment's promise holds for every config.

**Steps to reproduce**

No tokenizer or GPU is needed. This is the upstream test's config with `head_dim` 8 instead of 16. Save as `poc.py`:

```python
import sys, tempfile, torch
from safetensors.torch import save_file
from torchspec.models.draft.auto import AutoDraftModelConfig, AutoEagle3DraftModel
from torchspec.models.draft.keymap import to_export_keys
from torchspec.training.checkpoint import load_initial_draft_weights

cfg = dict(architectures=["LlamaForCausalLMEagle3"], model_type="llama", hidden_size=64,
           intermediate_size=128, num_hidden_layers=1, num_attention_heads=2, num_key_value_heads=1,
           head_dim=8, vocab_size=128, draft_vocab_size=128, max_position_embeddings=256,
           rms_norm_eps=1e-6, rope_theta=10000.0)
build = lambda c: AutoEagle3DraftModel.from_config(AutoDraftModelConfig.from_dict(c), torch_dtype=torch.float32)

eagle3 = build(cfg)
eagle4 = build({**cfg, "architectures": ["LlamaForCausalLMEagle4"], "attn_norm": True})
with tempfile.TemporaryDirectory() as d:
    save_file(to_export_keys(eagle3.state_dict()), f"{d}/model.safetensors")
    load_initial_draft_weights(eagle4, d)
print("loaded, eh_proj", tuple(eagle4.state_dict()["midlayer.eh_proj.weight"].shape))
```

```
python poc.py
# expected: loaded, eh_proj (64, 128)
# torchspec: RuntimeError: Incompatible initial LlamaForCausalLMEagle3 checkpoint: missing=[], unexpected=[], shape_mismatch=['midlayer.eh_proj.weight', ...]
```

**Actual output**

```
Traceback (most recent call last):
  File "poc.py", line 17, in <module>
    load_initial_draft_weights(eagle4, d)
  File "torchspec/training/checkpoint.py", line 299, in load_initial_draft_weights
    return _load_initial_weights(
           ^^^^^^^^^^^^^^^^^^^^^^
  File "torchspec/training/checkpoint.py", line 237, in _load_initial_weights
    raise error_type(
RuntimeError: Incompatible initial LlamaForCausalLMEagle3 checkpoint: missing=[], unexpected=[], shape_mismatch=['midlayer.eh_proj.weight', 'midlayer.self_attn.k_proj.weight', 'midlayer.self_attn.q_proj.weight', 'midlayer.self_attn.v_proj.weight']
```

**Expected output**

```
loaded, eh_proj (64, 128)
```

**Environment**

- TorchSpec main at c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9
- torch 2.15.0, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux x86_64 (Ubuntu, glibc 2.39)
- CPU only, no GPU

**Latest release**

The repository has no release tags (version.txt says 0.1.0). The code came in with a4e5c10 "feat: improve models (#200)".

**Related issues and PRs**

I searched open and closed issues and PRs. None reports this. Related: #200 (added `_upgrade_to_eagle4`), #197 and #168 (earlier warm-start work in `load_initial_draft_weights`).

A fix with a test is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
