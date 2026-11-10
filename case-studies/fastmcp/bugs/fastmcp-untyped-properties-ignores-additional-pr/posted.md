# Posted: https://github.com/PrefectHQ/fastmcp/issues/6124

Filed by alpaylan, 2026-10-28. Text as posted:

**json_schema_to_type drops additionalProperties keys when the schema has properties but no type**

### What happened?

An object schema that has `properties` but no `"type": "object"` ignores `additionalProperties: true`, so `json_schema_to_type` silently drops every undeclared key. With `"type": "object"` the same schema keeps them.

Through the client, a tool with output schema `{"properties": {"name": {"type": "string"}}, "additionalProperties": true}` returning `{"name": "x", "extra": 1}` gets `result.data == field_unknown(name='x')`, losing `extra`.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

for schema in (
    {"properties": {}, "additionalProperties": True},
    {"type": "object", "properties": {}, "additionalProperties": True},
):
    ta = TypeAdapter(json_schema_to_type(schema))
    print(ta.dump_python(ta.validate_python({"a": 1})))
```

Output:

```
{}
{'a': 1}
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
