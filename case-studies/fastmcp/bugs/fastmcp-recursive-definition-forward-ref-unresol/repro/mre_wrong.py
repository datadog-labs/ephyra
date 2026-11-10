from pydantic import TypeAdapter

from fastmcp.utilities.json_schema_type import json_schema_to_type

# Untitled definitions: Pet.owner is a $ref to Person.
schema = {
    "type": "object",
    "$defs": {
        "Person": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "pet": {"$ref": "#/$defs/Pet"},
            },
        },
        "Pet": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "owner": {"$ref": "#/$defs/Person"},
            },
        },
    },
    "properties": {"person": {"$ref": "#/$defs/Person"}},
}

Root = json_schema_to_type(schema)
value = TypeAdapter(Root).validate_python(
    {"person": {"name": "Ann", "pet": {"name": "Rex", "owner": {"name": "Bob", "pet": {"name": "Max"}}}}}
)
print(value.person.pet.owner)
