from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

# Node.wrapper.node refers back to Node (a valid recursive schema).
schema = {
    "type": "object",
    "$defs": {
        "Node": {
            "title": "Node",
            "type": "object",
            "properties": {
                "wrapper": {
                    "type": "object",
                    "properties": {"node": {"$ref": "#/$defs/Node"}},
                }
            },
        }
    },
    "properties": {"node": {"$ref": "#/$defs/Node"}},
}

Root = json_schema_to_type(schema)
print(TypeAdapter(Root).validate_python({}))
