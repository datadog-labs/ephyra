# DFlash draft with linear rope_scaling vs the same config in transformers (CPU only).
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
