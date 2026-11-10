// Usage: neg-literal-poc <policy.dw> <schema.cedarschema>
// Prints the original policy, its expanded_source(), and the Cedar policy
// each one lowers to. Exits 1 if the two lowerings differ, 0 if they match.
use dogwood_language::{LoweredPolicySet, ParsedPolicySet, PolicySchema, ServiceSchema};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&args[1]).unwrap();
    let schema = PolicySchema::from_cedarschema_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let svc = ServiceSchema::builder().build().unwrap();

    let parsed = ParsedPolicySet::parse(&src, &svc).unwrap();
    let rendered = parsed.policies().next().unwrap().expanded_source();
    let lower = |s: &str| {
        LoweredPolicySet::from_str(s, &svc, &schema).unwrap().as_cedar().to_string()
    };
    let (a, b) = (lower(&src), lower(&rendered));
    println!("original:          {}", src.trim().replace('\n', " "));
    println!("expanded_source(): {}", rendered.trim().replace('\n', " "));
    println!("original lowers to:  {}", a.trim().replace('\n', " "));
    println!("rendered lowers to:  {}", b.trim().replace('\n', " "));
    std::process::exit(if a == b { 0 } else { 1 });
}
