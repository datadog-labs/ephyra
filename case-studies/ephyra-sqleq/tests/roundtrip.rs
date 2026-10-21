//! Encoding a seed and decoding its own term gives the seed back, and every
//! drawn variant is valid on the schema.

use ephyra::{Gen, Source};
use ephyra_postgresql::{expr::Schema, probes::case_queries, cases::final_model};
use ephyra_sqleq::{fragment, lang, variants};

#[test]
fn seeds_round_trip_and_variants_are_valid() {
    let (mut seeds, mut undecodable, mut invalid) = (0, 0, 0);
    for s in 0..300u64 {
        let (case, qs) = case_queries().generate(&mut Source::new(s, (s % 101) as usize));
        let schema = Schema::of(&final_model(&case));
        for q in qs.iter().filter_map(fragment::seed) {
            seeds += 1;
            let enc = lang::encode(&q);
            // As SQL: a NaN literal is not equal to itself as a Value.
            assert_eq!(lang::decode(&enc.term, &enc).unwrap().sql(), q.sql(), "seed {s}");
            let (_, report) = variants::variants(&q, &schema, 4, 30, s);
            undecodable += report.undecodable.len();
            invalid += report.invalid.len();
        }
    }
    assert!(seeds > 300, "{seeds} seeds");
    assert_eq!((undecodable, invalid), (0, 0), "over {seeds} seeds");
}
