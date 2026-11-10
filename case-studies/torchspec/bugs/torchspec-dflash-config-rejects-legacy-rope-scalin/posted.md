**Description**

A Llama-3.1 target config stores RoPE in the legacy form: a top-level `rope_theta` plus `rope_scaling={"rope_type": "llama3", ...}`. `LlamaConfig` accepts this form. `DFlashConfig` given the same values raises:

```
KeyError: "Missing required keys in `rope_parameters` for 'rope_type'='llama3': {'rope_theta'}"
```

`DFlashConfig.__init__` (`torchspec/models/draft/dflash.py:69`) passes `rope_scaling` through `**kwargs` to `PretrainedConfig.__init__`. In transformers 5.x that call stores the dict as `rope_parameters` without merging `rope_theta` into it. It then validates the dict, and the llama3 validator requires a nested `rope_theta`.

TorchSpec copies the target's `rope_scaling` and `rope_theta` into the draft config (`param_mappings` in `torchspec/config/utils.py:151-153`). A Llama-3.1 target therefore leads straight to this input. The DSpark config in the same package already handles both names. Its comment says: "transformers 5.x aliases rope_scaling to rope_parameters (a property on PretrainedConfig), so both names must resolve to one normalized dict. An explicit legacy rope_scaling wins over rope_parameters" (`torchspec/models/draft/dspark.py:192-195`). `DFlashConfig` does not normalize the dict.

Expected: the config builds, as `LlamaConfig` does for the same values.

The same KeyError happens:
- for `DSparkConfig`, which subclasses `DFlashConfig`;
- through `AutoDraftModelConfig.from_dict({"architectures": ["DFlashDraftModel"], ...})`, the path a `draft_model_config` JSON file takes;
- with `rope_parameters={"rope_type": "llama3", ...}` and no nested `rope_theta`.

This is related to, but separate from, the AttributeError on YaRN `rope_scaling` (missing `max_position_embeddings`). That one comes from the field order in `DFlashConfig.__init__`. I checked that setting the fields before `super().__init__()` still gives this KeyError.

**Steps to reproduce**

CPU only, no GPU. No tokenizer files or downloads are needed.

```python
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
```

```
python poc.py
# transformers (LlamaConfig): LlamaConfig: ok
# torchspec (DFlashConfig): KeyError: "Missing required keys in `rope_parameters` for 'rope_type'='llama3': {'rope_theta'}"
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
  File "poc.py", line 13, in <module>
    DFlashConfig(rope_theta=500000.0, max_position_embeddings=131072, rope_scaling=llama3())
  File "torchspec/models/draft/dflash.py", line 69, in __init__
    super().__init__(tie_word_embeddings=tie_word_embeddings, **kwargs)
  File "huggingface_hub/dataclasses.py", line 276, in init_with_validate
    cls.validate(self)  # type: ignore [attr-defined]
  File "huggingface_hub/dataclasses.py", line 251, in validate
    validator(self)
  File "transformers/modeling_rope_utils.py", line 873, in validate_rope
    validation_fn(rope_parameters, ignore_keys=self.ignore_keys_at_rope_validation)
  File "transformers/modeling_rope_utils.py", line 1044, in _validate_llama3_rope_parameters
    self._check_received_keys(rope_type, received_keys, required_keys, ignore_keys=ignore_keys)
  File "transformers/modeling_rope_utils.py", line 1114, in _check_received_keys
    raise KeyError(f"Missing required keys in `rope_parameters` for 'rope_type'='{rope_type}': {missing_keys}")
KeyError: "Missing required keys in `rope_parameters` for 'rope_type'='llama3': {'rope_theta'}"
```

**Environment**

- TorchSpec commit c4fc3e0f6d9254b642e9def8edc71c58eb4a64f9 (current main)
- torch 2.15.0+cu132, transformers 5.20.2, tokenizers 0.23.3
- Python 3.12.3, Linux x86_64 (Ubuntu, glibc 2.39)
- CPU only, no GPU

`pyproject.toml` lists `transformers` without a version pin. I did not test transformers 4.x.

**Latest release**

The repository has no tags or GitHub releases, so I tested current main only. The code dates from the commit that added `DFlashConfig` (760a552, #64).

**Related issues and PRs**

- #207 (open): a YaRN `rope_scaling` crashes `build_rotary_embedding` in the Eagle3 draft. Different code and a different error.
- #166 (closed PR): resolves `rope_theta` from wherever the config stores it. It does not change `DFlashConfig.__init__`.
- #101 / #103 (closed): a null `rope_type`. Not this bug.

I found no existing issue for this crash.

A fix is available.

Found by an automated property-based tester (ephyra); this report was drafted with AI assistance.
