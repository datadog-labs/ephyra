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
