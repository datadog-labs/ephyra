// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Equivalent PostgreSQL queries for sqleq: seeds (a schema and a query)
//! from ephyra-postgresql, equivalent variants from an e-graph, and the
//! pair files and verdicts of sqleq's frontend, provers and fuzzer.

pub mod antirules;
pub mod check;
pub mod ddl;
pub mod fragment;
pub mod inputs;
pub mod lang;
pub mod minimize;
pub mod mutate;
pub mod onestep;
pub mod pgcheck;
pub mod relational;
pub mod sample;
pub mod tools;
pub mod variants;
pub mod witness;
