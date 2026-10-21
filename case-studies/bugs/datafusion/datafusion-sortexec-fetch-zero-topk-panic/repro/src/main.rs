// SELECT ... ORDER BY ... LIMIT 0 panics in TopKHeap::new when the
// `eliminate_limit` logical optimizer rule is not in the rule set.
use datafusion::error::Result;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = SessionContext::new();
    assert!(ctx.remove_optimizer_rule("eliminate_limit"));
    ctx.sql("CREATE TABLE t0(c0 BIGINT) AS VALUES (3), (1), (2)").await?;

    let df = ctx.sql("SELECT c0 FROM t0 ORDER BY c0 LIMIT 0").await?;
    println!("{}", df.clone().explain(false, false)?.to_string().await?);
    let batches = df.collect().await?;
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    println!("rows returned: {rows}");
    Ok(())
}
