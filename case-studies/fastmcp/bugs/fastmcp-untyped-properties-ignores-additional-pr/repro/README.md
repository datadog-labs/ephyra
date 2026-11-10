# Repro: json_schema_to_type drops extra keys for untyped `properties` + `additionalProperties: true`

Run with any Python that imports the fastmcp build under test:

    python mre.py

Bug present:
    {'properties': {}, 'additionalProperties': True} -> {}
    {'type': 'object', 'properties': {}, 'additionalProperties': True} -> {'a': 1}
    structured_content: {'name': 'x', 'extra': 1}
    data: field_unknown(name='x')

Bug absent:
    {'properties': {}, 'additionalProperties': True} -> {'a': 1}
    {'type': 'object', 'properties': {}, 'additionalProperties': True} -> {'a': 1}
    structured_content: {'name': 'x', 'extra': 1}
    data: name='x' extra=1

Checked 2026-10-28: reproduces on main 92c4c8d7, v4.1.4, 3.4.11. Not on the patched build.
