**Description**

Building a `DFlashConfig` or `DSparkConfig` with a YaRN `rope_scaling` raises:

```
AttributeError: 'DFlashConfig' object has no attribute 'max_position_embeddings'
```

`DFlashConfig.__init__` (`torchspec/models/draft/dflash.py:69`) calls `super().__init__(tie_word_embeddings=tie_word_embeddings, **kwargs)` first. It sets `self.max_position_embeddings`, `self.rope_theta` and the other fields only after that call. In transformers 5.x, `PretrainedConfig.__init__` validates the rope parameters. For YaRN, `_validate_yarn_rope_parameters` reads `self.max_position_embeddings`, which does not exist yet.

`DFlashConfig` takes `max_position_embeddings` as an argument and passes other keyword arguments, including `rope_scaling`, on to `PretrainedConfig`. `LlamaConfig` accepts the same values. The config should build, or reject the value with a clear error. When the GQA DFlash2 config does not support scaling, it says so: `raise ValueError("GQA DFlash2 training does not support rope_scaling")` (`torchspec/models/draft/dflash2.py:216`). `K3DSparkConfig` takes `rope_scaling` / `rope_parameters` out of kwargs itself. It does not crash.

The same crash happens:
- with `rope_parameters={..., "rope_type": "yarn"}` instead of `rope_scaling`;
- through `AutoDraftModelConfig.from_dict({"architectures": ["DFlashDraftModel"], ...})`, which is the path a `draft_model_config` JSON file takes;
- for `DSparkConfig`, which subclasses `DFlashConfig`.

Linear scaling and the default config (no `rope_scaling`) build fine.

**Steps to reproduce**

CPU only, no GPU. No tokenizer files or downloads are needed.

```python
# poc.py
from transformers import LlamaConfig
from torchspec import DFlashConfig

yarn = {"rope_type": "yarn", "factor": 4.0, "original_max_position_embeddings": 32}

LlamaConfig(max_position_embeddings=128, rope_scaling=yarn)
print("LlamaConfig: ok")
DFlashConfig(max_position_embeddings=128, rope_scaling=yarn)
print("DFlashConfig: ok")
```

```
python poc.py
# transformers (LlamaConfig): LlamaConfig: ok
# torchspec (DFlashConfig): AttributeError: 'DFlashConfig' object has no attribute 'max_position_embeddings'
```

Expected output:

```
LlamaConfig: ok
DFlashConfig: ok
```

Actual output (paths shortened, caret lines removed):

```
LlamaConfig: ok
Traceback (most recent call last):
  File "poc.py", line 9, in <module>
    DFlashConfig(max_position_embeddings=128, rope_scaling=yarn)
  File "torchspec/models/draft/dflash.py", line 69, in __init__
    super().__init__(tie_word_embeddings=tie_word_embeddings, **kwargs)
  File "huggingface_hub/dataclasses.py", line 276, in init_with_validate
    cls.validate(self)  # type: ignore [attr-defined]
  File "huggingface_hub/dataclasses.py", line 251, in validate
    validator(self)
  File "transformers/modeling_rope_utils.py", line 873, in validate_rope
    validation_fn(rope_parameters, ignore_keys=self.ignore_keys_at_rope_validation)
  File "transformers/modeling_rope_utils.py", line 972, in _validate_yarn_rope_parameters
    implicit_factor = self.max_position_embeddings / original_max_position_embeddings
  File "transformers/configuration_utils.py", line 652, in __getattribute__
    return super().__getattribute__(key)
  File "transformers/integrations/heterogeneity/configuration_utils.py", line 312, in __getattribute__
    return super().__getattribute__(key)
AttributeError: 'DFlashConfig' object has no attribute 'max_position_embeddings'
```

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux x86_64 (Ubuntu, glibc 2.39)
- CPU only, no GPU

`pyproject.toml` lists `transformers` without a version pin, so an install that resolves transformers 5.x gets the rope validation that triggers this. I did not test transformers 4.x.

**Latest release**

The repository has no tags or GitHub releases, so I tested current main only. The ordering dates from the commit that added `DFlashConfig` (760a552, #64).

**Related issues and PRs**

- #207 (open): a YaRN `rope_scaling` crashes `build_rotary_embedding` with a TypeError in the Eagle3 draft. It is different code with a different error, so not a duplicate.
- #166 (closed): resolves `rope_theta` from wherever the config stores it. It does not touch `DFlashConfig.__init__`.

I found no existing issue for this crash.

A fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
