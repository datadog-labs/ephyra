**Title:** `upsert_entities` keeps a stale ancestor when the batch contains the same entity twice (regression in 4.11.0)

### Before opening, please confirm:

- [x] I have [searched for duplicate or closed issues](https://github.com/cedar-policy/cedar/issues?q=is%3Aissue+).
- [x] I have read the guide for [submitting bug reports](https://github.com/cedar-policy/cedar/blob/main/CONTRIBUTING.md#bug-reports).
- [x] I have done my best to include a minimal, self-contained set of instructions for consistently reproducing the issue.

### Describe the bug

`cedar_policy::Entities::upsert_entities` forwards to the core `Entities::upsert_entities`, which documents that "when a duplicate is encountered, the value is overwritten by the latest version". When a batch names an entity twice, and a later entity in the same batch removes a parent edge, a descendant keeps the ancestor it reached through that edge. So `principal in …` stays true and a revoked membership still grants access.

In the reproduction below, alice is in team, team is in org (not stored) and admins, and admins is in root. One batch updates org twice and takes admins out of root:

- **Expected:** after the batch, nothing leads from alice to `Group::"root"`, so `permit(principal in Group::"root", action, resource);` denies her.
- **Actual:** `Entities::ancestors(alice)` still contains `Group::"root"`, and the request is allowed.

The request is denied when the first version of `Org::"o"` is left out, and also when the same final entities are built with `Entities::from_entities`.

**Versions.** Fails on main (0b80d019) and on 4.11.0 and 4.13.0. 4.10.0 gives the right answer. The bug came with #2317 (cd1a4041, the transitive closure rewrite). No modifications, default features.

**Cause.** When `upsert_entities` replaces an entity already in the map, it does two things. First it removes the entity's uid and the old entity's ancestors from every descendant's indirect ancestors. Then `repair_tc`, which only adds edges, recomputes the touched entities. That is complete when the old entity has its closure. When the old entity is an earlier version from the same batch, however, its ancestors are only its parents, because the closure is computed after the loop.

Here, replacing `Org::"o" in [admins]` removes admins from alice's indirect ancestors, but not admins' ancestor root. When `Group::"admins"` is upserted next, `is_descendant_of` no longer finds alice among its descendants, so root is never removed.

**Fix.** Apply only the last version of each entity in the batch, as the documentation describes, while still checking every version against the schema. Then every entity replaced is one stored before the call, with its closure. The patch at the end applies to main with `git am`. It includes a CHANGELOG entry and a unit test (`upsert_entities_repeated_in_batch`, which fails without the fix). I'm happy to open it as a PR.

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

#[test]
fn upsert_batch_naming_an_entity_twice_revokes_membership() {
    // alice in team; team in org (not stored) and admins; admins in root.
    let entities = Entities::from_entities(
        [
            entity(r#"User::"alice""#, &[r#"Team::"t""#]),
            entity(r#"Team::"t""#, &[r#"Org::"o""#, r#"Group::"admins""#]),
            entity(r#"Group::"admins""#, &[r#"Group::"root""#]),
        ],
        None,
    )
    .unwrap()
    // One batch: org under admins, then org under nothing (the latest
    // version wins), and admins leaves root.
    .upsert_entities(
        [
            entity(r#"Org::"o""#, &[r#"Group::"admins""#]),
            entity(r#"Org::"o""#, &[]),
            entity(r#"Group::"admins""#, &[]),
        ],
        None,
    )
    .unwrap();

    // Nothing leads from alice to root any more. Allow on 4.11.0 and later.
    let policies = r#"permit(principal in Group::"root", action, resource);"#;
    assert_eq!(decide(policies, &entities), Decision::Deny);
}
```

### Patch

Applies to main (0b80d019) with `git am`: save the block below as `fix.patch` and run `git am fix.patch`.

<details>
<summary>fix.patch</summary>

```diff
From 1fe3af74f9e6eaffb9215672412840fced3a62f2 Mon Sep 17 00:00:00 2001
From: =?UTF-8?q?Alperen=20Kele=C5=9F?= <alpkeles99@gmail.com>
Date: Thu, 8 Oct 2026 14:52:00 -0400
Subject: [PATCH] Apply only the last version of an entity repeated in
 `upsert_entities`
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

`upsert_entities` documents that when the same entity is given more than
once, the latest version overwrites the earlier ones. Replacing a stored
entity first removes the entity and its old ancestors from the indirect
ancestors of its descendants, then `repair_tc` recomputes the touched
entities, adding edges only. When the entity being replaced is an
earlier version from the same collection, its closure has not been
computed yet, so its ancestors are only its parents. Its descendants
then lose the parents but keep the parents' ancestors. A later entity in
the same collection that removes one of those edges no longer finds
those descendants, so they keep an ancestor that the update removed. For
example, a user keeps a group membership that was revoked, and
`principal in Group::"..."` stays true.

Apply only the last version of each entity, while still checking every
version against the schema, so that every entity replaced is one stored
before the call, with its closure computed.

Introduced in 4.11.0 by the transitive closure rewrite (#2317).

Signed-off-by: Alperen Keleş <alpkeles99@gmail.com>
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
---
 cedar-policy-core/src/entities.rs | 71 ++++++++++++++++++++++++++++++-
 cedar-policy/CHANGELOG.md         |  6 +++
 2 files changed, 75 insertions(+), 2 deletions(-)

diff --git a/cedar-policy-core/src/entities.rs b/cedar-policy-core/src/entities.rs
index c5440a01..28c4f03a 100644
--- a/cedar-policy-core/src/entities.rs
+++ b/cedar-policy-core/src/entities.rs
@@ -237,11 +237,25 @@ impl Entities {
         extensions: &Extensions<'_>,
     ) -> Result<Self> {
         let checker = schema.map(|schema| EntitySchemaConformanceChecker::new(schema, extensions));
+        let collection: Vec<Arc<Entity>> = collection.into_iter().collect();
+        // Only the last version of an entity given more than once is applied.
+        // Overwriting an earlier version from this same collection would strip
+        // only that version's parents from its descendants, as its transitive
+        // closure is not computed yet, and leave stale ancestors behind.
+        let last_version: HashMap<&EntityUID, usize> = collection
+            .iter()
+            .enumerate()
+            .map(|(i, entity)| (entity.uid(), i))
+            .collect();
         let mut entities_touched: HashSet<EntityUID> = HashSet::new();
-        for entity in collection.into_iter() {
+        for (i, entity) in collection.iter().enumerate() {
             if let Some(checker) = checker.as_ref() {
-                checker.validate_entity(&entity)?;
+                checker.validate_entity(entity)?;
             }
+            if last_version.get(entity.uid()) != Some(&i) {
+                continue;
+            }
+            let entity = Arc::clone(entity);
             let uid = entity.uid().clone();
             // If overwriting an existing entity, strip stale TC edges from its descendants
             if let Some(old_entity) = self.entities.get(&uid) {
@@ -2528,6 +2542,59 @@ mod entities_tests {
         assert!(!f.is_descendant_of(&eid));
     }
 
+    #[test]
+    fn upsert_entities_repeated_in_batch() {
+        // Original Hierarchy
+        // A -> T -> O (O is not an entity in the store)
+        // A -> T -> G -> R
+        let aid = EntityUID::with_eid("A");
+        let tid = EntityUID::with_eid("T");
+        let oid = EntityUID::with_eid("O");
+        let gid = EntityUID::with_eid("G");
+        let rid = EntityUID::with_eid("R");
+        let mut a = Entity::with_uid(aid.clone());
+        a.add_parent(tid.clone());
+        let mut t = Entity::with_uid(tid);
+        t.add_parent(oid.clone());
+        t.add_parent(gid.clone());
+        let mut g = Entity::with_uid(gid.clone());
+        g.add_parent(rid.clone());
+
+        // One batch: O -> G, then O with no parents (the latest version wins),
+        // and G with no parents
+        let mut o_first = Entity::with_uid(oid.clone());
+        o_first.add_parent(gid.clone());
+        let updates = vec![
+            o_first,
+            Entity::with_uid(oid),
+            Entity::with_uid(gid.clone()),
+        ]
+        .into_iter()
+        .map(Arc::new)
+        .collect::<Vec<_>>();
+        let entities = Entities::from_entities(
+            vec![a, t, g],
+            None::<&NoEntitiesSchema>,
+            TCComputation::ComputeNow,
+            Extensions::all_available(),
+        )
+        .expect("Failed to construct entities")
+        .upsert_entities(
+            updates,
+            None::<&NoEntitiesSchema>,
+            TCComputation::ComputeNow,
+            Extensions::all_available(),
+        )
+        .expect("Failed to upsert entities");
+        // Post-Update Hierarchy
+        // A -> T -> O, A -> T -> G
+
+        let a = entities.entity(&aid).unwrap();
+        assert!(a.is_descendant_of(&gid));
+        // Assert that there is no longer a path from A to R
+        assert!(!a.is_descendant_of(&rid));
+    }
+
     // ---- Vec-of-test-cases for add/remove/upsert with TC repair ----
     //
     // NOTE: `test_remove_entities` and `test_upsert_entities` above exercise
diff --git a/cedar-policy/CHANGELOG.md b/cedar-policy/CHANGELOG.md
index 3f31bb90..e7367494 100644
--- a/cedar-policy/CHANGELOG.md
+++ b/cedar-policy/CHANGELOG.md
@@ -24,6 +24,12 @@ Cedar Language Version: TBD
   parses the string and renders the result back out to check that the input was normalized. The same
   optimization was applied to `from_normalized_str()` for `InternalName`, `UnreservedId` and `AnyId`.
 
+### Fixed
+
+- Fixed `Entities::upsert_entities` keeping stale ancestors when the same entity is given more than once.
+  An entity could keep an ancestor that the update removed, so that `in` remained true. Only the last
+  version of each entity is now applied, as documented (introduced in 4.11.0).
+
 ## [4.13.0] - 2026-09-15
 
 Cedar Language Version: 4.5
-- 
2.54.0 (Apple Git-157)

```

</details>
