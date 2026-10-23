# Bugs: cedar

The bugs the case studies found in cedar. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [`add_entities` compares a new entity's parents with the stored entity's transitive closure: a declared parent can be dropped, and identical entities are refused](cedar-add-entities-closure-duplicate/) | open | [link](https://github.com/cedar-policy/cedar/issues/2629) |
| [`upsert_entities` keeps a stale ancestor when the batch contains the same entity twice (regression in 4.11.0)](cedar-upsert-repeated-entity-stale-ancestor/) | fixed | [link](https://github.com/cedar-policy/cedar/issues/2606) |
