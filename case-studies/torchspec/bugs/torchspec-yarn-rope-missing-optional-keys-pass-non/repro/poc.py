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
