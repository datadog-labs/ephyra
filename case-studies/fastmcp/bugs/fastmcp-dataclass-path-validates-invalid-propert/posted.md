# Posted: https://github.com/PrefectHQ/fastmcp/issues/6123

Filed by alpaylan, 2026-10-28. Text as posted:

**json_schema_to_type rejects a valid object when a property's default doesn't match its own type**

### What happened?

For an object schema without `additionalProperties`, a property whose `default` doesn't satisfy its own type makes `json_schema_to_type` reject instances that omit that property.

`{"type": "object", "properties": {"name": {"type": "string", "default": null}}}` rejects `{}`. Pydantic emits exactly this schema for `name: str = None`. The same schema with `"additionalProperties": true` accepts `{}`, so the two object paths disagree. Through the client, a tool with this output schema returning `{}` gets `result.data == None`.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

prop = {"name": {"type": "string", "default": None}}
for schema in (
    {"type": "object", "properties": prop, "additionalProperties": True},
    {"type": "object", "properties": prop},
):
    try:
        TypeAdapter(json_schema_to_type(schema)).validate_python({})
        print("ok")
    except Exception as e:
        print(type(e).__name__, str(e).splitlines()[1:3])
```

Output:

```
ok
ValidationError ['name', '  Input should be a valid string [type=string_type, input_value=None, input_type=NoneType]']
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
