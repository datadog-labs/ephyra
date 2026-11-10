# poc.py
from transformers import LlamaConfig
from torchspec import DFlashConfig


def llama3():  # a fresh dict per call: LlamaConfig adds rope_theta to the dict it is given
    return {"rope_type": "llama3", "factor": 8.0, "low_freq_factor": 1.0,
            "high_freq_factor": 4.0, "original_max_position_embeddings": 8192}


LlamaConfig(rope_theta=500000.0, max_position_embeddings=131072, rope_scaling=llama3())
print("LlamaConfig: ok")
DFlashConfig(rope_theta=500000.0, max_position_embeddings=131072, rope_scaling=llama3())
print("DFlashConfig: ok")
