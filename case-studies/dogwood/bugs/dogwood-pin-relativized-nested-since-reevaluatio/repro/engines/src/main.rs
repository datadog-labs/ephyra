// usage: engines <global|slice|partition> policy.dw schema.cedarschema trace.log
use dogwood_language::{
    Authorizer, Decision, InMemoryTemporalEngine, LoweredPolicySet, PolicySchema, ServiceSchema,
    parse_trace,
};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let rd = |p: &str| std::fs::read_to_string(p).unwrap();
    let (policy, schema, trace) = (rd(&a[2]), rd(&a[3]), rd(&a[4]));
    let ps = PolicySchema::from_cedarschema_str(&schema).unwrap();
    let lowered = LoweredPolicySet::from_str(&policy, &ServiceSchema::defaults(), &ps).unwrap();
    let events = parse_trace(&trace).unwrap();
    let mut auth = match a[1].as_str() {
        "global" => Authorizer::new(lowered),
        "slice" => Authorizer::builder(lowered)
            .temporal_engine(InMemoryTemporalEngine::new().slice_leaves())
            .build()
            .unwrap(),
        "partition" => Authorizer::builder(lowered).partition_temporal().build().unwrap(),
        e => panic!("unknown engine {e}"),
    };
    let t0 = std::time::Instant::now();
    for e in &events {
        if let Some(r) = auth.is_authorized(e) {
            let v = if r.decision() == Decision::Allow { "ALLOW" } else { "DENY" };
            println!("@{} {v}", e.timestamp());
        }
    }
    eprintln!("{} engine: {:.2?}", a[1], t0.elapsed());
}
