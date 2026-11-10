// usage: dw-engines <policy.dw> <schema.cedarschema> <trace.log>
// Prints the Validator result, then the verdict of each decision point under
// the global, slice_leaves and partitioned engines (default event schema).
use dogwood_language::*;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let policy = std::fs::read_to_string(&a[1]).unwrap();
    let schema = std::fs::read_to_string(&a[2]).unwrap();
    let trace = std::fs::read_to_string(&a[3]).unwrap();
    let ps = PolicySchema::from_cedarschema_str(&schema).unwrap();
    let ss = ServiceSchema::defaults();
    let lower = || LoweredPolicySet::from_str(&policy, &ss, &ps).unwrap();
    let lps = lower();
    let v = Validator::new().validate(&lps);
    println!("validator errors: {}", v.validation_errors().count());
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
