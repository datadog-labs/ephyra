from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

schema = {"type": "array", "prefixItems": [{"type": "string"}], "items": {"type": "number"}}
print(TypeAdapter(json_schema_to_type(schema)).validate_python(["a"]))
