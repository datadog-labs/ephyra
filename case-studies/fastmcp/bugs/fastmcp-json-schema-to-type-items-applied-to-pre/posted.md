# Posted: https://github.com/PrefectHQ/fastmcp/issues/6122

Filed by alpaylan, 2026-10-28. Text as posted:

**json_schema_to_type applies items to the prefixItems positions**

### What happened?

For an array schema with both `prefixItems` and `items`, `json_schema_to_type` applies `items` to every element, including the `prefixItems` positions, unless `minItems == maxItems == len(prefixItems)`. In JSON Schema, `items` only applies to elements after the `prefixItems`.

So `["a"]` is rejected for `{"type": "array", "prefixItems": [{"type": "string"}], "items": {"type": "number"}}`, and `items: false` rejects every non-empty array. Through the client, a tool output with this shape gets `result.data == None`.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

schema = {"type": "array", "prefixItems": [{"type": "string"}], "items": {"type": "number"}}
print(TypeAdapter(json_schema_to_type(schema)).validate_python(["a"]))
```

Output:

```
pydantic_core._pydantic_core.ValidationError: 1 validation error for list[float]
0
  Input should be a valid number, unable to parse string as a number [type=float_parsing, input_value='a', input_type=str]
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
