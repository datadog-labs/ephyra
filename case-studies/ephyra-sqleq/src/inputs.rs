// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The inputs of a seed number, as the driver draws them: the case's final
//! model, and per query in sqleq's fragment the seed and its variants.

use ephyra::{Gen, Source};
use ephyra_postgresql::{
    cases::{final_model, Case},
    expr::{Query, Schema},
    model::Model,
    probes::case_queries,
};

use crate::{fragment, variants};

pub struct Input {
    pub case: Case,
    pub model: Model,
    pub schema: Schema,
    /// (seed, its variants), one per query in the fragment.
    pub seeds: Vec<(Query, Vec<Query>)>,
}

pub const VARIANTS: usize = 4;
pub const SAMPLES: usize = 60;

pub fn input(s: u64) -> Input {
    let (case, qs) = case_queries().generate(&mut Source::new(s, (s % 101) as usize));
    let model = final_model(&case);
    let schema = Schema::of(&model);
    let seeds = qs.iter().filter_map(fragment::seed).map(|q| {
        let (vs, _) = variants::variants(&q, &schema, VARIANTS, SAMPLES, s);
        (q, vs)
    }).collect();
    Input { case, model, schema, seeds }
}
