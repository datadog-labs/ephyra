//! Stateful cases: a table and a script of statements over it. Scripts are
//! generated against a live SQLite, so later statements use what earlier
//! ones left behind (how many rows there are, which values they hold), and
//! they are traces: causality analysis resumes one after any prefix.

use std::{fmt, ops::ControlFlow};

use ephyra::{Gen, Source, Trace};

use crate::{
    ast::{Expr, Literal, Query, Type, create},
    engine::{self, Outcome, Sqlite, Turso, Value},
    generate::{
        TYPES, expr, filter_fits, fits, literal, query, query_fits, row_fits, shrink_expr,
        shrink_literal, shrink_query, value,
    },
};

pub const MAX_STEPS: usize = 200;
/// Doubling stops once the table has this many rows, so scripts stay fast.
const MAX_ROWS: i64 = 512;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Insert(Vec<Literal>),
    /// `INSERT INTO t SELECT * FROM t`: the table doubles.
    Double,
    Update {
        column: usize,
        value: Expr,
        filter: Option<Expr>,
    },
    Delete(Option<Expr>),
    /// `CREATE INDEX IF NOT EXISTS`, one index per column at most.
    Index(usize),
    Select(Query),
}

impl Step {
    pub fn sql(&self, arity: usize) -> String {
        let filter =
            |p: &Option<Expr>| p.as_ref().map(|p| format!(" WHERE {p}")).unwrap_or_default();
        match self {
            Step::Insert(row) => {
                let values: Vec<String> = row.iter().map(|v| v.to_string()).collect();
                format!("INSERT INTO t VALUES ({})", values.join(", "))
            },
            Step::Double => "INSERT INTO t SELECT * FROM t".to_string(),
            Step::Update { column, value, filter: p } => {
                format!("UPDATE t SET c{column} = {value}{}", filter(p))
            },
            Step::Delete(p) => format!("DELETE FROM t{}", filter(p)),
            Step::Index(c) => format!("CREATE INDEX IF NOT EXISTS i{c} ON t(c{c})"),
            Step::Select(q) => q.sql(arity),
        }
    }

    /// Whether the step changes rows (and so `changes()`).
    fn changes_rows(&self) -> bool {
        !matches!(self, Step::Index(_) | Step::Select(_))
    }
}

#[derive(Clone, PartialEq)]
pub struct Script {
    pub schema: Vec<Type>,
    /// How the workload is weighted; resuming and mutating keep it.
    pub mode: Mode,
    pub steps: Vec<Step>,
}

/// What a script's workload leans toward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Build,
    Write,
    Read,
}

const MODES: [Mode; 3] = [Mode::Build, Mode::Write, Mode::Read];

/// Where a step falls in its script: the first 15% set the table up, the
/// last 10% read it back, the rest is the workload.
#[derive(Clone, Copy)]
enum Phase {
    Setup,
    Workload,
    Verify,
}

fn phase(i: usize, len: usize) -> Phase {
    if i < len * 15 / 100 {
        Phase::Setup
    } else if len >= 10 && i >= len - len / 10 {
        Phase::Verify
    } else {
        Phase::Workload
    }
}

/// Weights of insert, double, update, delete, index and select.
fn weights(phase: Phase, mode: Mode) -> [u32; 6] {
    match (phase, mode) {
        (Phase::Setup, _) => [70, 5, 0, 0, 10, 15],
        (Phase::Workload, Mode::Build) => [40, 10, 10, 5, 5, 30],
        (Phase::Workload, Mode::Write) => [20, 5, 30, 20, 5, 20],
        (Phase::Workload, Mode::Read) => [10, 3, 7, 5, 5, 70],
        (Phase::Verify, _) => [0, 0, 0, 0, 0, 100],
    }
}

/// The operators a targeted predicate compares a column with a value by.
const COMPARE: [&str; 7] = ["=", "<>", "<", "<=", ">", ">=", "LIKE"];

impl fmt::Display for Script {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{};", create(&self.schema))?;
        for step in &self.steps {
            write!(f, "\n{};", step.sql(self.schema.len()))?;
        }
        Ok(())
    }
}

/// Counterexamples print as the SQL script.
impl fmt::Debug for Script {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "\n{self}")
    }
}

/// A script is well formed when the table has 1 to 4 columns, it has at
/// most [`MAX_STEPS`] steps, and every step fits the table: inserted rows
/// match the schema, references name columns, filters are within
/// `MAX_DEPTH`. The schema never changes, so any subsequence of a well
/// formed script is well formed.
#[ephyra::generator]
pub fn script_well_formed(script: &Script) -> bool {
    let arity = script.schema.len();
    let fits_step = |step: &Step| match step {
        Step::Insert(row) => row_fits(row, &script.schema),
        Step::Double => true,
        Step::Update { column, value, filter } => {
            *column < arity
                && filter_fits(value, arity)
                && filter.as_ref().is_none_or(|p| filter_fits(p, arity))
        },
        Step::Delete(filter) => filter.as_ref().is_none_or(|p| filter_fits(p, arity)),
        Step::Index(c) => *c < arity,
        Step::Select(q) => query_fits(q, arity),
    };
    (1..=4).contains(&arity)
        && script.steps.len() <= MAX_STEPS
        && script.steps.iter().all(fits_step)
}

#[ephyra::generator(script_well_formed, metrics = script_metrics)]
pub fn scripts() -> impl Trace<Value = Script> {
    ScriptGen
}

pub struct ScriptGen;

/// What generation knows about the table. The indexes follow from the
/// steps alone; the row count and the values recent writes stored are read
/// back from the SQLite after each step.
struct Model {
    indexed: Vec<bool>,
    rows: i64,
    /// (column, value) for recent writes, oldest first, at most 16.
    recent: Vec<(usize, Literal)>,
}

/// The SQLite a script is generated against, and the model kept beside it.
/// Generation only reads the SQLite with deterministic queries, so a script
/// depends on its seed alone.
struct World {
    db: Sqlite,
    schema: Vec<Type>,
    model: Model,
}

impl World {
    fn new(schema: &[Type]) -> Self {
        let db = Sqlite::open();
        db.run(&create(schema));
        let model = Model { indexed: vec![false; schema.len()], rows: 0, recent: Vec::new() };
        World { db, schema: schema.to_vec(), model }
    }

    /// Run `step` and bring the model up to date.
    fn run(&mut self, step: &Step) {
        self.db.run(&step.sql(self.schema.len()));
        self.model.rows = self.count();
        let written: Vec<(usize, Literal)> = match step {
            Step::Insert(row) => row.iter().cloned().enumerate().collect(),
            Step::Update { column, value: Expr::Literal(v), .. } => vec![(*column, v.clone())],
            Step::Delete(Some(p)) => compared(p),
            Step::Index(c) => {
                self.model.indexed[*c] = true;
                vec![]
            },
            _ => vec![],
        };
        let recent = &mut self.model.recent;
        recent.extend(written.into_iter().filter(|(_, v)| *v != Literal::Null));
        let excess = recent.len().saturating_sub(16);
        recent.drain(..excess);
    }

    fn count(&self) -> i64 {
        match self.db.run("SELECT count(*) FROM t") {
            Outcome::Rows(rows) => match rows.first().and_then(|r| r.first()) {
                Some(Value::Integer(n)) => *n,
                _ => 0,
            },
            Outcome::Error(_) => 0,
        }
    }

    /// A value column `c` holds in some row, when the table has rows.
    fn sample(&self, g: &mut Source, c: usize) -> Option<Literal> {
        if self.model.rows <= 0 {
            return None;
        }
        let k = g.int(0..self.model.rows);
        let sql = format!("SELECT c{c} FROM t ORDER BY rowid LIMIT 1 OFFSET {k}");
        let Outcome::Rows(found) = self.db.run(&sql) else { return None };
        match found.first()?.first()? {
            Value::Null => Some(Literal::Null),
            Value::Integer(n) => Some(Literal::Integer(*n)),
            Value::Real(x) if x.is_finite() => Some(Literal::Real(*x)),
            Value::Text(s) => Some(Literal::Text(s.clone())),
            Value::Real(_) | Value::Blob(_) => None,
        }
    }

    /// A value to compare column `c` with or write into it: a recent write
    /// to it, a value it holds, or a fresh one of its type.
    fn value_for(&self, g: &mut Source, c: usize) -> Literal {
        let recent: Vec<&Literal> =
            self.model.recent.iter().filter(|(k, _)| *k == c).map(|(_, v)| v).collect();
        if !recent.is_empty() && g.bool_with(0.4) {
            return (*g.pick(&recent)).clone();
        }
        if g.bool_with(0.8)
            && let Some(v) = self.sample(g, c)
        {
            return v;
        }
        value(g, self.schema[c])
    }

    /// A column to aim at, indexed ones more often.
    fn column(&self, g: &mut Source) -> usize {
        let indexed: Vec<usize> =
            (0..self.schema.len()).filter(|&c| self.model.indexed[c]).collect();
        if !indexed.is_empty() && g.bool_with(0.6) {
            *g.pick(&indexed)
        } else {
            g.index(self.schema.len())
        }
    }

    /// `c op value` for a column and one of its values. Not NULL: a
    /// comparison with NULL keeps nothing; NULLs are found with `IS NULL`.
    fn target(&self, g: &mut Source) -> Expr {
        let c = self.column(g);
        let op = *g.pick(&COMPARE);
        let v = match self.value_for(g, c) {
            Literal::Null => literal(g, self.schema[c]),
            v => v,
        };
        Expr::Binary(op, Box::new(Expr::Column(c)), Box::new(Expr::Literal(v)))
    }

    /// A filter that meets the data: mostly a comparison of a column with
    /// one of its values, alone or combined, else a random expression whose
    /// column comparisons are aimed the same way.
    fn predicate(&self, g: &mut Source) -> Expr {
        match g.int(0..10u8) {
            0..4 => self.target(g),
            4..6 => {
                let op = *g.pick(&["AND", "OR"]);
                let depth = g.int(1..=2usize);
                let other = expr(g, &self.schema, depth);
                Expr::Binary(op, Box::new(self.target(g)), Box::new(other))
            },
            6 => Expr::IsNull(Box::new(Expr::Column(self.column(g)))),
            _ => {
                let depth = g.int(1..=4usize);
                let mut p = expr(g, &self.schema, depth);
                self.aim(&mut p, g);
                p
            },
        }
    }

    /// Point most of `e`'s column-to-literal comparisons at values of that
    /// column.
    fn aim(&self, e: &mut Expr, g: &mut Source) {
        match e {
            Expr::Binary(op, a, b) if COMPARE.contains(op) => {
                if let (Expr::Column(c), Expr::Literal(l)) | (Expr::Literal(l), Expr::Column(c)) =
                    (a.as_mut(), b.as_mut())
                {
                    if g.bool_with(0.7) {
                        match self.value_for(g, *c) {
                            Literal::Null => {},
                            v => *l = v,
                        }
                    }
                    return;
                }
                self.aim(a, g);
                self.aim(b, g);
            },
            Expr::Binary(_, a, b) => {
                self.aim(a, g);
                self.aim(b, g);
            },
            Expr::Prefix(_, a) | Expr::IsNull(a) => self.aim(a, g),
            Expr::Column(_) | Expr::Literal(_) => {},
        }
    }

    /// The next step: its kind weighted by phase, mode and what the table
    /// holds now, its contents aimed at the data.
    fn step(&self, g: &mut Source, phase: Phase, mode: Mode) -> Step {
        let rows = self.model.rows;
        let mut w = weights(phase, mode);
        if rows == 0 {
            // Nothing to update, delete or find yet.
            w = [w[0] + 20, 0, 0, 0, w[4], w[5] / 4];
        }
        if !(8..MAX_ROWS).contains(&rows) {
            // Doubling a tiny table only makes copies of a few rows.
            w[1] = 0;
        }
        if rows > MAX_ROWS / 2 {
            w[0] /= 2;
            w[3] *= 2;
        }
        if self.model.indexed.iter().all(|&i| i) {
            w[4] = 0;
        }
        let total: u32 = w.iter().sum();
        let mut roll = g.int(0..total.max(1));
        let kind = w.iter().position(|&x| {
            let hit = roll < x;
            roll = roll.saturating_sub(x);
            hit
        });
        match kind {
            // Mostly fresh values, so the table has distinct ones to find;
            // some repeats, so equality has duplicates to find.
            Some(0) => {
                let row = (0..self.schema.len()).map(|c| {
                    let ty = self.schema[c];
                    let v = g.bool_with(0.25).then(|| self.value_for(g, c));
                    v.filter(|v| fits(v, ty)).unwrap_or_else(|| value(g, ty))
                });
                Step::Insert(row.collect())
            },
            Some(1) => Step::Double,
            Some(2) => {
                let column = self.column(g);
                let value = if g.bool() {
                    Expr::Literal(self.value_for(g, column))
                } else {
                    let depth = g.int(1..=2usize);
                    let mut v = expr(g, &self.schema, depth);
                    self.aim(&mut v, g);
                    v
                };
                Step::Update { column, value, filter: g.bool_with(0.8).then(|| self.predicate(g)) }
            },
            Some(3) => Step::Delete(g.bool_with(0.9).then(|| self.predicate(g))),
            Some(4) => {
                let free: Vec<usize> =
                    (0..self.schema.len()).filter(|&c| !self.model.indexed[c]).collect();
                Step::Index(*g.pick(&free))
            },
            _ => {
                let mut q = query(g, &self.schema);
                if q.filter.is_some() {
                    q.filter = Some(self.predicate(g));
                }
                if q.limit.is_some() {
                    q.limit = Some(g.int(0..=rows.clamp(0, 2 * MAX_ROWS) as u32 + 1));
                }
                Step::Select(q)
            },
        }
    }
}

/// The (column, literal) pairs `p` compares directly.
fn compared(p: &Expr) -> Vec<(usize, Literal)> {
    match p {
        Expr::Binary(op, a, b) if COMPARE.contains(op) => match (a.as_ref(), b.as_ref()) {
            (Expr::Column(c), Expr::Literal(l)) | (Expr::Literal(l), Expr::Column(c)) => {
                vec![(*c, l.clone())]
            },
            _ => [compared(a), compared(b)].concat(),
        },
        _ => p.children().into_iter().flat_map(compared).collect(),
    }
}

impl ScriptGen {
    /// `prefix`, continued against the state it leaves until the script has
    /// `len` steps.
    fn extend(
        &self,
        schema: &[Type],
        mode: Mode,
        prefix: &[Step],
        len: usize,
        g: &mut Source,
    ) -> Script {
        let mut world = World::new(schema);
        prefix.iter().for_each(|s| world.run(s));
        let mut steps = prefix.to_vec();
        while steps.len() < len {
            let step = world.step(g, phase(steps.len(), len), mode);
            world.run(&step);
            steps.push(step);
        }
        Script { schema: schema.to_vec(), mode, steps }
    }
}

impl Gen for ScriptGen {
    type Value = Script;

    fn generate(&self, g: &mut Source) -> Script {
        let arity = g.int(1..=4usize);
        let schema: Vec<Type> = (0..arity).map(|_| *g.pick(&TYPES)).collect();
        // Sizes grow with the logarithm of the test count (about 11 after
        // 2,000 tests, 13 after 10,000), so the length scales by sixteen.
        let len = g.int(1..=(8 + 16 * g.size()).min(MAX_STEPS));
        let mode = *g.pick(&MODES);
        self.extend(&schema, mode, &[], len, g)
    }

    /// Halves, then single steps, then each step's own shrinks. Every
    /// candidate is a subsequence or has one step simplified; the schema
    /// is fixed, so all stay well formed.
    fn shrink(
        &self,
        script: &Script,
        visit: &mut dyn FnMut(Script) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let n = script.steps.len();
        let with = |steps: Vec<Step>| Script { steps, ..script.clone() };
        if n > 1 {
            visit(with(script.steps[..n / 2].to_vec()))?;
            visit(with(script.steps[n / 2..].to_vec()))?;
        }
        for i in 0..n {
            let mut steps = script.steps.clone();
            steps.remove(i);
            visit(with(steps))?;
        }
        for (i, step) in script.steps.iter().enumerate() {
            for smaller in shrink_step(step) {
                let mut steps = script.steps.clone();
                steps[i] = smaller;
                visit(with(steps))?;
            }
        }
        ControlFlow::Continue(())
    }

    /// Cut the script somewhere and continue it from the state there, up to
    /// sixteen steps longer than it was.
    fn mutate(&self, script: &Script, g: &mut Source) -> Script {
        let cut = g.int(0..=script.steps.len());
        let len = (script.steps.len() + g.int(0..=16usize)).clamp(1, MAX_STEPS);
        self.extend(&script.schema, script.mode, &script.steps[..cut], len, g)
    }
}

impl Trace for ScriptGen {
    fn steps(&self, script: &Script) -> usize {
        script.steps.len()
    }

    /// The first `prefix` steps, continued to the script's length.
    fn resume(&self, script: &Script, prefix: usize, g: &mut Source) -> Script {
        let prefix = &script.steps[..prefix.min(script.steps.len())];
        self.extend(&script.schema, script.mode, prefix, script.steps.len(), g)
    }
}

fn shrink_step(step: &Step) -> Vec<Step> {
    match step {
        Step::Insert(row) => (0..row.len())
            .flat_map(|c| {
                shrink_literal(&row[c]).into_iter().map(move |v| {
                    let mut row = row.clone();
                    row[c] = v;
                    Step::Insert(row)
                })
            })
            .collect(),
        Step::Double | Step::Index(_) => vec![],
        Step::Update { column, value, filter } => {
            let update =
                |value: Expr, filter: Option<Expr>| Step::Update { column: *column, value, filter };
            let mut out = Vec::new();
            if let Some(p) = filter {
                out.push(update(value.clone(), None));
                out.extend(shrink_expr(p).into_iter().map(|p| update(value.clone(), Some(p))));
            }
            out.extend(shrink_expr(value).into_iter().map(|v| update(v, filter.clone())));
            out
        },
        Step::Delete(None) => vec![],
        Step::Delete(Some(p)) => {
            let mut out = vec![Step::Delete(None)];
            out.extend(shrink_expr(p).into_iter().map(|p| Step::Delete(Some(p))));
            out
        },
        Step::Select(q) => shrink_query(q).into_iter().map(Step::Select).collect(),
    }
}

/// Run `script` in Turso and in SQLite, one step at a time, and check after
/// every step that they agree: the same rows or both an error, the same
/// number of rows changed by a write, and the same table afterwards.
/// Returns how many steps failed in both.
pub fn differential(script: &Script) -> usize {
    let (turso, sqlite) = (Turso::open(), Sqlite::open());
    let arity = script.schema.len();
    let contents = "SELECT rowid, * FROM t ORDER BY rowid";
    let create = create(&script.schema);
    assert!(engine::agree(true, &turso.run(&create), &sqlite.run(&create)), "{create}");
    let mut errors = 0;
    for (i, step) in script.steps.iter().enumerate() {
        let sql = step.sql(arity);
        let (a, b) = (turso.run(&sql), sqlite.run(&sql));
        let ordered = matches!(step, Step::Select(q) if q.order.is_some());
        assert!(
            engine::agree(ordered, &a, &b),
            "step {}: {sql}\nturso: {a:?}\nsqlite: {b:?}",
            i + 1
        );
        match a {
            Outcome::Error(_) => errors += 1,
            Outcome::Rows(_) if step.changes_rows() => {
                let (x, y) = (turso.changes(), sqlite.changes());
                assert_eq!(x, y, "step {}: {sql}\nrows changed differ", i + 1);
            },
            Outcome::Rows(_) => {},
        }
        if !matches!(step, Step::Select(_)) {
            let (a, b) = (turso.run(contents), sqlite.run(contents));
            assert!(
                engine::agree(true, &a, &b),
                "after step {}: {sql}\nturso: {a:?}\nsqlite: {b:?}",
                i + 1
            );
        }
    }
    errors
}

/// - steps: raw count.
/// - one count per kind of step: inserts, doubles, updates, deletes,
///   indexes, selects.
/// - filtered writes: updates and deletes with a `WHERE`, over all updates
///   and deletes; 0 with none.
pub fn script_metrics(script: &Script, _size: usize) -> Vec<(&'static str, f64)> {
    let count = |kind: fn(&Step) -> bool| script.steps.iter().filter(|s| kind(s)).count() as f64;
    let writes = count(|s| matches!(s, Step::Update { .. } | Step::Delete(_)));
    let filtered =
        count(|s| matches!(s, Step::Update { filter: Some(_), .. } | Step::Delete(Some(_))));
    vec![
        ("steps", script.steps.len() as f64),
        ("inserts", count(|s| matches!(s, Step::Insert(_)))),
        ("doubles", count(|s| matches!(s, Step::Double))),
        ("updates", count(|s| matches!(s, Step::Update { .. }))),
        ("deletes", count(|s| matches!(s, Step::Delete(_)))),
        ("indexes", count(|s| matches!(s, Step::Index(_)))),
        ("selects", count(|s| matches!(s, Step::Select(_)))),
        ("filtered writes", if writes == 0.0 { 0.0 } else { filtered / writes }),
    ]
}
