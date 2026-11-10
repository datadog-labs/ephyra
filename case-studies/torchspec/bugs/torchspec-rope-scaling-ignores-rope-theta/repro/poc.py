"""build_rotary_embedding drops rope_theta on the linear, dynamic and mrope branches.

CPU only, no downloads. Run from a TorchSpec checkout: python poc.py
Exit 1 if any branch differs from the configured base / transformers' RoPE.
"""
import torch
from transformers import LlamaConfig
from transformers.models.llama.modeling_llama import LlamaRotaryEmbedding
from torchspec.models.draft.llama3_eagle import build_rotary_embedding

THETA = 1e6
bad = False
for rope_type in ["linear", "dynamic", "mrope"]:
    scaling = {"rope_type": rope_type}
    if rope_type != "mrope":
        scaling["factor"] = 2.0
    cfg = LlamaConfig(hidden_size=8, num_attention_heads=1, num_key_value_heads=1, head_dim=8,
                      max_position_embeddings=64, rope_theta=THETA, rope_scaling=scaling)
    draft = build_rotary_embedding(cfg, dim=8, max_position_embeddings=64)
    line = f"{rope_type:8} torchspec base={draft.base}"
    ok = draft.base == THETA
    if rope_type != "mrope":  # transformers has no 'mrope' rope_type to compare against
        cos_hf, _ = LlamaRotaryEmbedding(cfg)(torch.zeros(1, 64, 8), torch.arange(64)[None])
        diff = (draft.cos_cached[0, 0, :64].float() - cos_hf[0]).abs().max().item()
        line += f"  max |cos - transformers cos| over positions 0..63: {diff:.4f}"
        ok = ok and diff < 1e-4
    print(line, "ok" if ok else "WRONG")
    bad |= not ok
raise SystemExit(1 if bad else 0)
