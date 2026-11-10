# DFlashConfig crashes on a valid YaRN rope_scaling; LlamaConfig accepts the same values.
from transformers import LlamaConfig
from torchspec import DFlashConfig

yarn = {"rope_type": "yarn", "factor": 4.0, "original_max_position_embeddings": 32}

LlamaConfig(max_position_embeddings=128, rope_scaling=yarn)
print("LlamaConfig: ok")
DFlashConfig(max_position_embeddings=128, rope_scaling=yarn)
print("DFlashConfig: ok")
