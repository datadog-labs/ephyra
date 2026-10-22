// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Run the property and print the report, suspects included.

fn main() {
    let report = locate_demo::check_addition();
    println!("{report}");
    if report.locate.as_ref().is_none_or(|l| l.suspects.is_empty()) {
        std::process::exit(1);
    }
}
