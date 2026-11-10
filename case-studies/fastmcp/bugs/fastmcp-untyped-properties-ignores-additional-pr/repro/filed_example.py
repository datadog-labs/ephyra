from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

for schema in (
    {"properties": {}, "additionalProperties": True},
    {"type": "object", "properties": {}, "additionalProperties": True},
):
    ta = TypeAdapter(json_schema_to_type(schema))
    print(ta.dump_python(ta.validate_python({"a": 1})))
