// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Updates of an `Entities` against the model in `ephyra_cedar::hierarchy`,
//! and the bugs that property found, as explicit cases.

use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityUid, PolicySet, Request,
};

use ephyra_cedar::hierarchy::{self, Model, Op, apply, reachable, step};

/// After every call of a trace, Cedar and the model agree on whether it
/// succeeds and on every entity's ancestors.
#[ephyra::test(gen = hierarchy::traces())]
fn entity_updates_match_model(ops: Vec<Op>) {
    let mut model = Model::new();
    let mut es = Entities::empty();
    for (i, op) in ops.iter().enumerate() {
        let expected = step(&model, op);
        let got = apply(es.clone(), op);
        assert_eq!(expected.is_some(), got.is_some(), "call {i} succeeds");
        let (Some(next), Some(got)) = (expected, got) else { continue };
        for id in 0..hierarchy::IDS {
            let want = next.contains_key(&id).then(|| reachable(&next, id));
            let have = got
                .ancestors(&hierarchy::uid(id))
                .map(|a| a.map(|u| u.id().unescaped().parse::<u8>().unwrap()).collect());
            assert_eq!(want, have, "ancestors of {id} after call {i}");
        }
        model = next;
        es = got;
    }
}

// The bugs it found, as explicit cases on users and groups.

fn uid(s: &str) -> EntityUid {
    EntityUid::from_str(s).unwrap()
}

fn entity(id: &str, parents: &[&str]) -> Entity {
    Entity::new_no_attrs(uid(id), parents.iter().map(|p| uid(p)).collect())
}

/// alice in eng, eng in all.
fn org() -> Entities {
    Entities::from_entities(
        [
            entity(r#"User::"alice""#, &[r#"Group::"eng""#]),
            entity(r#"Group::"eng""#, &[r#"Group::"all""#]),
            entity(r#"Group::"all""#, &[]),
        ],
        None,
    )
    .unwrap()
}

/// The documentation allows adding an entity identical to one already stored;
/// it is refused when that entity has an ancestor beyond its parents.
#[test]
fn readding_an_identical_entity_is_accepted() {
    let added = org().add_entities([entity(r#"User::"alice""#, &[r#"Group::"eng""#])], None);
    assert!(added.is_ok(), "{:?}", added.err());
}

/// An entity whose parents differ from the stored one's is not identical, so
/// adding it must fail as a duplicate or keep its parents. When its parents
/// equal the stored one's closure, it is taken as identical and dropped:
/// alice's declared parent `all` is lost once eng leaves all.
#[test]
fn added_parents_are_kept() {
    let Ok(es) = org()
        .add_entities([entity(r#"User::"alice""#, &[r#"Group::"eng""#, r#"Group::"all""#])], None)
    else {
        return;
    };
    let es = es.upsert_entities([entity(r#"Group::"eng""#, &[])], None).unwrap();

    let policies =
        PolicySet::from_str(r#"permit(principal in Group::"all", action, resource);"#).unwrap();
    let request = Request::new(
        uid(r#"User::"alice""#),
        uid(r#"Action::"view""#),
        uid(r#"Doc::"d""#),
        Context::empty(),
        None,
    )
    .unwrap();
    let alice = es.get(&uid(r#"User::"alice""#)).unwrap().to_json_string().unwrap();
    assert_eq!(
        Authorizer::new().is_authorized(&request, &policies, &es).decision(),
        Decision::Allow,
        "alice after the add and the upsert: {alice}"
    );
}

/// An upsert batch may name an entity twice ("the value is overwritten by the
/// latest version"). Replacing the batch's own first version strips only that
/// version's parents from its descendants, not their closure, so a later
/// entity in the batch misses those descendants and leaves a stale ancestor.
#[test]
fn upsert_batch_with_a_repeated_entity_drops_stale_ancestors() {
    // alice in team; team in org (not stored yet) and admins; admins in root.
    let org = || {
        Entities::from_entities(
            [
                entity(r#"User::"alice""#, &[r#"Team::"t""#]),
                entity(r#"Team::"t""#, &[r#"Org::"o""#, r#"Group::"admins""#]),
                entity(r#"Group::"admins""#, &[r#"Group::"root""#]),
            ],
            None,
        )
        .unwrap()
    };
    let policies =
        PolicySet::from_str(r#"permit(principal in Group::"root", action, resource);"#).unwrap();
    let request = Request::new(
        uid(r#"User::"alice""#),
        uid(r#"Action::"view""#),
        uid(r#"Doc::"d""#),
        Context::empty(),
        None,
    )
    .unwrap();
    let decide = |batch: Vec<Entity>| {
        let es = org().upsert_entities(batch, None).unwrap();
        let ancestors: Vec<String> =
            es.ancestors(&uid(r#"User::"alice""#)).unwrap().map(ToString::to_string).collect();
        (Authorizer::new().is_authorized(&request, &policies, &es).decision(), ancestors)
    };

    // The entities after the batch, built at once: alice is not in root.
    let fresh = Entities::from_entities(
        [
            entity(r#"User::"alice""#, &[r#"Team::"t""#]),
            entity(r#"Team::"t""#, &[r#"Org::"o""#, r#"Group::"admins""#]),
            entity(r#"Org::"o""#, &[]),
            entity(r#"Group::"admins""#, &[]),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        Authorizer::new().is_authorized(&request, &policies, &fresh).decision(),
        Decision::Deny
    );

    // Without org's first version, admins leaving root takes alice out of root.
    let (decision, ancestors) =
        decide(vec![entity(r#"Org::"o""#, &[]), entity(r#"Group::"admins""#, &[])]);
    assert_eq!(decision, Decision::Deny, "alice's ancestors: {ancestors:?}");

    // One batch: org first under admins, then under nothing; admins leaves root.
    let (decision, ancestors) = decide(vec![
        entity(r#"Org::"o""#, &[r#"Group::"admins""#]),
        entity(r#"Org::"o""#, &[]),
        entity(r#"Group::"admins""#, &[]),
    ]);
    assert_eq!(decision, Decision::Deny, "alice's ancestors: {ancestors:?}");
}
