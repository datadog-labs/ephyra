SET disabled_optimizers='expression_rewriter';
SELECT quantile_disc(1, 0.5), max(0.5::DOUBLE);
EXPLAIN (SQL) SELECT quantile_disc(1, 0.5), max(0.5::DOUBLE);
SET debug_verify_statement='explain_sql';
SELECT quantile_disc(1, 0.5), max(0.5::DOUBLE);
