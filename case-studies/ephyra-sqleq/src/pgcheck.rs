//! PostgreSQL as a judge: a seed and its variants on the case's own data
//! (its setup and statements, as the oracles run them), results compared
//! as bags of values PostgreSQL compares equal (`sqlancer::canon`). A
//! difference means a rule (or the decoder) is wrong. Plan-dependent errors
//! (overflow, out of range) on either side are skipped, as the oracles do.

use ephyra_postgresql::{
    cases::Case,
    expr::Query,
    pg::with_session,
    sqlancer::{canon, expected_error, known_error, prepare, run, Canon},
};

/// What one seed's comparisons gave.
#[derive(Debug, Default)]
pub struct Verdict {
    pub compared: usize,
    pub skipped: usize,
    /// (variant SQL, why): a result differs, or only one side fails.
    pub differ: Vec<(String, String)>,
}

fn bag(rows: &[Vec<ephyra_postgresql::pg::Cell>]) -> Vec<Vec<Canon>> {
    let mut b: Vec<Vec<Canon>> = rows.iter().map(|r| r.iter().map(canon).collect()).collect();
    b.sort();
    b
}

pub fn agree(case: &Case, seed: &Query, variants: &[Query]) -> Result<Verdict, String> {
    with_session(|s| {
        prepare(s, case)?;
        let mut v = Verdict::default();
        let skip = |e: &str| expected_error(e) || known_error(e) || e.contains("statement timeout");
        let orig = match run(s, &seed.sql()) {
            Ok(rows) => bag(&rows),
            Err(e) if skip(&e) => {
                v.skipped += variants.len();
                return Ok(v);
            },
            Err(e) => return Err(format!("the seed fails: {e}\n  {}", seed.sql())),
        };
        for q in variants {
            match run(s, &q.sql()) {
                Ok(rows) if bag(&rows) == orig => v.compared += 1,
                Ok(_) => v.differ.push((q.sql(), "results differ".into())),
                Err(e) if skip(&e) => v.skipped += 1,
                Err(e) => v.differ.push((q.sql(), format!("the variant fails: {e}"))),
            }
        }
        Ok(v)
    })
}
