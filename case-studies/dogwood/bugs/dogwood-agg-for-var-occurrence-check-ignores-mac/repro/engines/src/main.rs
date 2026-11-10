// usage: dw-engines <policy.dw> <schema.cedarschema> <providers.json> <trace.log>
// Lowers the policy set (default event schema plus the providers), then prints
// the verdict of each decision point under the global, slice_leaves and
// partitioned engines.
use dogwood_language::*;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rd = |i: usize| std::fs::read_to_string(&a[i]).unwrap();
    let (policy, schema, providers, trace) = (rd(1), rd(2), rd(3), rd(4));
    let ps = PolicySchema::from_cedarschema_str(&schema).unwrap();
    let ss = || {
        ServiceSchema::builder()
            .providers(ProviderDeclarations::from_json(&providers).unwrap())
            .build()
            .unwrap()
    };
    let lower = || LoweredPolicySet::from_str(&policy, &ss(), &ps);
    if let Err(e) = lower() {
        println!("lowering failed: {e}");
        std::process::exit(1);
    }
    let lower = || lower().unwrap();
    let events = parse_trace(&trace).unwrap();
    let mut engines: Vec<(&str, Authorizer)> = vec![
        ("global", Authorizer::new(lower())),
        ("slice_leaves", Authorizer::builder(lower())
            .temporal_engine(InMemoryTemporalEngine::new().slice_leaves()).build().unwrap()),
        ("partitioned", Authorizer::builder(lower()).partition_temporal().build().unwrap()),
    ];
    for (name, auth) in engines.iter_mut() {
        let verdicts: Vec<String> = events.iter()
            .filter_map(|e| auth.is_authorized(e).map(|r| format!("@{} {:?}", e.timestamp(), r.decision())))
            .collect();
        println!("{name}: {}", verdicts.join(", "));
    }
}
