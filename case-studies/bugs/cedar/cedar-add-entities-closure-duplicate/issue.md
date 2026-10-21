**Title:** `add_entities` compares a new entity's parents with the stored entity's transitive closure: a declared parent can be dropped, and identical entities are refused

### Before opening, please confirm:

- [x] I have [searched for duplicate or closed issues](https://github.com/cedar-policy/cedar/issues?q=is%3Aissue+).
- [x] I have read the guide for [submitting bug reports](https://github.com/cedar-policy/cedar/blob/main/CONTRIBUTING.md#bug-reports).
- [x] I have done my best to include a minimal, self-contained set of instructions for consistently reproducing the issue.

### Describe the bug

`Entities::add_entities` documents that it fails with `EntitiesError::Duplicate` when "there is an entity in `entities` with the same Entity UID as a non-identical entity in this structure". The check compares the new entity's parents with the stored entity's ancestors. Those ancestors include the closure computed when the entity was added, which gives two wrong results.

Both reproductions start from: alice in eng, and eng in contractors.

1. **A different entity is accepted as identical, and its new parent is dropped.**
   - *Steps:* add alice with parents [eng, contractors]. The call succeeds, but alice keeps her stored parents [eng]. Then eng leaves contractors.
   - *Expected:* alice is still in contractors, because she was declared a direct member, so `permit(principal, action, resource); forbid(principal in Group::"contractors", action, resource);` denies her. Alternatively, the add fails as a duplicate.
   - *Actual:* she is no longer in contractors, and she is **allowed**. With a permit on contractors instead, she would be wrongly denied.
2. **An identical entity is refused.**
   - *Steps:* re-add alice with parents [eng], exactly as stored.
   - *Expected:* `Ok`, with nothing changed.
   - *Actual:* `Err(Duplicate(User::"alice"))`. Any incremental loading with overlapping batches hits this as soon as an entity has a grandparent.

As a result, the same pair of entities is identical or not depending on whether the closure has been computed:

| | `Entities::from_entities` | `add_entities` on a store holding the first |
|---|---|---|
| alice in [eng], given twice | ok | `Duplicate` |
| alice in [eng], then alice in [eng, contractors] | `Duplicate` | ok (the second is dropped) |

The 4.5.1 changelog (#1725) describes identical entities as having the same "identifiers, attributes, ancestors, and tags". The table shows that "ancestors" means different things before and after the closure is computed.

**Versions.** Both fail on main (4d1ef853) and on 4.5.0, 4.10.0, 4.11.0, 4.13.0 and 4.14.0. By the code, they go back to #1473 (4.4.0), which allowed identical duplicates. No modifications, default features.

**Cause.** `update_entity_map` decides "identical" with `Entity::deep_eq`, whose documentation says it "does not test that they have the same _direct_ parents, only that they have the same overall ancestor set". That suits comparing two `Entities` whose closures are computed. Here, though, it compares an entity as given, whose ancestors are only its parents, with one already stored, whose ancestors are its closure.

**Fix.** Compare what the two entities declare: id, attributes, tags and direct parents, through a new crate-private `Entity::declared_eq`. `deep_eq` is unchanged. The patch at the end applies to main with `git am`. It includes a CHANGELOG entry and two unit tests (`add_consistent_duplicate_with_indirect_ancestors` and `add_inconsistent_duplicate_parents`, which both fail without the fix). I'm happy to open it as a PR.

One consequence to weigh, related to #2341: `to_json` writes each entity's closure as its parents. An entity read back from that output and added to a store that holds the original now differs from it and is a `Duplicate`. Today it is accepted and dropped.

Found by property-based tests that run random sequences of `add_entities`, `upsert_entities` and `remove_entities` and compare every entity's ancestors with reachability recomputed from the declared parents.

### Reproduction steps

```rust
// Put your code below this line.
use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityUid, PolicySet, Request,
};

fn uid(s: &str) -> EntityUid {
    EntityUid::from_str(s).unwrap()
}

fn entity(id: &str, parents: &[&str]) -> Entity {
    Entity::new_no_attrs(uid(id), parents.iter().map(|p| uid(p)).collect())
}

fn decide(policies: &str, entities: &Entities) -> Decision {
    let policies = PolicySet::from_str(policies).unwrap();
    let request = Request::new(
        uid(r#"User::"alice""#),
        uid(r#"Action::"view""#),
        uid(r#"Doc::"d""#),
        Context::empty(),
        None,
    )
    .unwrap();
    Authorizer::new().is_authorized(&request, &policies, entities).decision()
}

/// alice in eng, eng in contractors.
fn stored() -> Entities {
    Entities::from_entities(
        [
            entity(r#"User::"alice""#, &[r#"Group::"eng""#]),
            entity(r#"Group::"eng""#, &[r#"Group::"contractors""#]),
            entity(r#"Group::"contractors""#, &[]),
        ],
        None,
    )
    .unwrap()
}

#[test]
fn added_parent_is_kept() {
    // alice is now declared directly in contractors as well. The entity is
    // not identical to the stored one, so the add may fail as a duplicate;
    // if it succeeds, the new parent must be kept.
    let Ok(entities) = stored().add_entities(
        [entity(r#"User::"alice""#, &[r#"Group::"eng""#, r#"Group::"contractors""#])],
        None,
    ) else {
        return;
    };
    // eng leaves contractors; alice is still a contractor directly.
    let entities = entities.upsert_entities([entity(r#"Group::"eng""#, &[])], None).unwrap();

    let policies = r#"
        permit(principal, action, resource);
        forbid(principal in Group::"contractors", action, resource);
    "#;
    // Allow on 4.4.0 and later.
    assert_eq!(decide(policies, &entities), Decision::Deny);
}

#[test]
fn identical_entity_can_be_added_again() {
    // Err(Duplicate(User::"alice")) on 4.4.0 and later.
    let added = stored().add_entities([entity(r#"User::"alice""#, &[r#"Group::"eng""#])], None);
    assert!(added.is_ok(), "{:?}", added.err());
}
```

### Patch

Applies to main (4d1ef853) with `git am`: save the block below as `fix.patch` and run `git am fix.patch`.

<details>
<summary>fix.patch</summary>

```diff
From 082b9d26f440ef965426e13e5bf1efcba124ec43 Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Tue, 20 Oct 2026 16:44:41 -0400
Subject: [PATCH] Compare declared parents when `add_entities` checks for
 duplicates
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

`add_entities` rejects an entity whose id is already stored unless the
two are identical, and keeps the stored one when they are. It decided
"identical" with `Entity::deep_eq`, which compares ancestor sets, not
direct parents. So it compared the new entity, whose ancestors are its
parents, with the stored entity, whose ancestors include its transitive
closure:

- An entity identical to the stored one was rejected as a duplicate as
  soon as the stored one had an indirect ancestor.
- An entity whose parents differed from the stored one's, but matched
  its closure, was accepted as identical and dropped. Its declared
  parents were lost, and after later updates `in` gave a different
  answer than for the same entities built at once.

The same pair of entities was therefore identical or not depending on
whether the closure had been computed: `from_entities` accepted what
`add_entities` rejected, and the reverse.

Compare what the two entities declare instead (id, attributes, tags and
parents), through a new crate-private `Entity::declared_eq`. `deep_eq`,
which compares entities whose closures are computed, is unchanged.

Signed-off-by: Alperen Keleş <alpkeles99@gmail.com>
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
---
 cedar-policy-core/src/ast/entity.rs | 13 ++++++
 cedar-policy-core/src/entities.rs   | 72 ++++++++++++++++++++++++++---
 cedar-policy/CHANGELOG.md           |  6 +++
 3 files changed, 85 insertions(+), 6 deletions(-)

diff --git a/cedar-policy-core/src/ast/entity.rs b/cedar-policy-core/src/ast/entity.rs
index d1de5e2e..73997a39 100644
--- a/cedar-policy-core/src/ast/entity.rs
+++ b/cedar-policy-core/src/ast/entity.rs
@@ -686,6 +686,19 @@ impl Entity {
                 == (other.ancestors().collect::<HashSet<_>>())
     }
 
+    /// Test if two `Entity` objects declare the same entity: the same UID,
+    /// attributes, attribute values, tags and _direct_ parents.
+    ///
+    /// Unlike [`Entity::deep_eq`], ancestors added by computing the transitive
+    /// closure are not compared, so an entity as given is equal to itself after
+    /// the closure of its hierarchy was computed.
+    pub(crate) fn declared_eq(&self, other: &Self) -> bool {
+        self.uid == other.uid
+            && self.attrs == other.attrs
+            && self.tags == other.tags
+            && self.parents == other.parents
+    }
+
     /// Mark the given `UID` as an indirect ancestor of this `Entity`
     ///
     /// The given `UID` will not be added as an indirecty ancestor if
diff --git a/cedar-policy-core/src/entities.rs b/cedar-policy-core/src/entities.rs
index c9907a79..ef2c7cc7 100644
--- a/cedar-policy-core/src/entities.rs
+++ b/cedar-policy-core/src/entities.rs
@@ -548,9 +548,9 @@ fn create_entity_map(
 
 /// Adds an entry to the specified map associating the EntityUID of the specified entity
 /// to the specified entity. Checks whether there is an entity already in the map
-/// with the same EntityUID as the specified entity. If such an entity is found and is
-/// not structurally equal to the specified entity produces an error. Otherwise,
-/// if a structurally equal entity is found, the state of the map is unchanged.
+/// with the same EntityUID as the specified entity. If such an entity is found and
+/// does not declare the same data as the specified entity produces an error. Otherwise,
+/// if an identical entity is found, the state of the map is unchanged.
 fn update_entity_map(
     map: &mut HashMap<EntityUID, Arc<Entity>>,
     entity: Arc<Entity>,
@@ -561,9 +561,10 @@ fn update_entity_map(
             if allow_override {
                 occupied_entry.insert(entity);
             } else {
-                // Check whether the occupying entity is structurally equal to the
-                // entity being processed
-                if !entity.deep_eq(occupied_entry.get()) {
+                // Check whether the occupying entity declares the same data as the
+                // entity being processed. Its ancestors may already include those
+                // computed for the transitive closure, so they are not compared.
+                if !entity.declared_eq(occupied_entry.get()) {
                     let entry = occupied_entry.remove_entry();
                     return Err(EntitiesError::duplicate(entry.0));
                 }
@@ -1175,6 +1176,65 @@ mod json_parsing_tests {
         assert_matches!(err, EntitiesError::Duplicate(d) => assert_eq!(d.euid(), &r#"Test::"jeff""#.parse().unwrap()));
     }
 
+    /// jeff in team, team in org: jeff's ancestors in the structure include org
+    fn jeff_in_team_in_org(parser: &EntityJsonParser<'_, '_>) -> Entities {
+        let json = serde_json::json!([
+            {"uid":{ "type" : "Test", "id" : "jeff" }, "attrs" : {}, "parents" : [{ "type" : "Test", "id" : "team" }]},
+            {"uid":{ "type" : "Test", "id" : "team" }, "attrs" : {}, "parents" : [{ "type" : "Test", "id" : "org" }]},
+            {"uid":{ "type" : "Test", "id" : "org" }, "attrs" : {}, "parents" : []}]);
+        parser
+            .from_json_value(json)
+            .unwrap_or_else(|e| panic!("{:?}", &miette::Report::new(e)))
+    }
+
+    #[test]
+    fn add_consistent_duplicate_with_indirect_ancestors() {
+        let parser: EntityJsonParser<'_, '_> =
+            EntityJsonParser::new(None, Extensions::all_available(), TCComputation::ComputeNow);
+        // The same jeff as in the structure, whose ancestors there also include org
+        let new = serde_json::json!([
+            {"uid":{ "type" : "Test", "id" : "jeff" }, "attrs" : {}, "parents" : [{ "type" : "Test", "id" : "team" }]}]);
+        let addl_entities = parser
+            .iter_from_json_value(new)
+            .unwrap_or_else(|e| panic!("{:?}", &miette::Report::new(e)))
+            .map(Arc::new);
+        let original = jeff_in_team_in_org(&parser);
+        let original_size = original.entities.len();
+        let es = original
+            .add_entities(
+                addl_entities,
+                None::<&NoEntitiesSchema>,
+                TCComputation::ComputeNow,
+                Extensions::all_available(),
+            )
+            .unwrap();
+        assert_eq!(es.entities.len(), original_size);
+    }
+
+    #[test]
+    fn add_inconsistent_duplicate_parents() {
+        let parser: EntityJsonParser<'_, '_> =
+            EntityJsonParser::new(None, Extensions::all_available(), TCComputation::ComputeNow);
+        // jeff directly in org as well: parents that differ from the structure's
+        // jeff, though they equal his ancestors there
+        let new = serde_json::json!([
+            {"uid":{ "type" : "Test", "id" : "jeff" }, "attrs" : {}, "parents" : [{ "type" : "Test", "id" : "team" }, { "type" : "Test", "id" : "org" }]}]);
+        let addl_entities = parser
+            .iter_from_json_value(new)
+            .unwrap_or_else(|e| panic!("{:?}", &miette::Report::new(e)))
+            .map(Arc::new);
+        let err = jeff_in_team_in_org(&parser)
+            .add_entities(
+                addl_entities,
+                None::<&NoEntitiesSchema>,
+                TCComputation::ComputeNow,
+                Extensions::all_available(),
+            )
+            .err()
+            .unwrap();
+        assert_matches!(err, EntitiesError::Duplicate(d) => assert_eq!(d.euid(), &r#"Test::"jeff""#.parse().unwrap()));
+    }
+
     #[test]
     fn simple_entities_correct() {
         let parser: EntityJsonParser<'_, '_> =
diff --git a/cedar-policy/CHANGELOG.md b/cedar-policy/CHANGELOG.md
index 6f37d442..fe9f2063 100644
--- a/cedar-policy/CHANGELOG.md
+++ b/cedar-policy/CHANGELOG.md
@@ -19,6 +19,12 @@ Cedar Language Version: TBD
 - Fixed `Entities::upsert_entities` keeping a stale ancestor when the same entity is given more than
   once in one batch.
 
+- Fixed `Entities::add_entities` and the `add_entities_from_json_*` functions comparing a new entity's parents
+  with the transitive closure of the stored entity with the same id. An identical entity was rejected as a
+  duplicate when the stored one had indirect ancestors, and an entity whose parents differed but matched that
+  closure was accepted as identical and its parents dropped. Entities are now identical when they have the same
+  id, attributes, tags and parents.
+
 ## [4.14.0] - 2026-10-17
 
 Cedar Language Version: 4.6
```

</details>
