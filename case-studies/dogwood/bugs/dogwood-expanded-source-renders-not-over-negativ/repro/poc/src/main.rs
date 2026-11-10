// Usage: not-neg-literal-poc <policy.dw> <schema.cedarschema>
// Prints the original policy, its expanded_source(), and what each one lowers
// to. Exits 1 if the rendered text does not lower, or lowers differently.
use dogwood_language::{LoweredPolicySet, ParsedPolicySet, PolicySchema, ServiceSchema};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&args[1]).unwrap();
    let schema = PolicySchema::from_cedarschema_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let svc = ServiceSchema::builder().build().unwrap();

    let parsed = ParsedPolicySet::parse(&src, &svc).unwrap();
    let rendered = parsed.policies().next().unwrap().expanded_source();
    let lower = |s: &str| match LoweredPolicySet::from_str(s, &svc, &schema) {
        Ok(set) => Ok(set.as_cedar().to_string()),
        Err(e) => Err(format!("{e}")),
    };
    let (a, b) = (lower(&src), lower(&rendered));
    let show = |r: &Result<String, String>| match r {
        Ok(s) => s.trim().replace('\n', " "),
        Err(e) => format!("ERROR: {}", e.trim().replace('\n', " ")),
    };
    println!("original:          {}", src.trim().replace('\n', " "));
    println!("expanded_source(): {}", rendered.trim().replace('\n', " "));
    println!("original lowers to:  {}", show(&a));
    println!("rendered lowers to:  {}", show(&b));
    std::process::exit(if a.is_ok() && a == b { 0 } else { 1 });
}
