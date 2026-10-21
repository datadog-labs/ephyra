// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! `ephyra-datadog dashboard`: lay out the dashboard for the ephyra metrics
//! in a Datadog org, and print it, create it, or update an existing one.
//!
//! Reads `DD_API_KEY`, `DD_APP_KEY` and `DD_SITE` (default `datadoghq.com`).

use std::{
    process::ExitCode,
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

use {
    ephyra_datadog::dashboard::{
        Schema,
        dashboard,
        discovery_queries,
    },
    serde_json::Value,
};

const USAGE: &str = "\
usage: ephyra-datadog dashboard [options]

Find the properties, features, labels, covers and observations that reached
Datadog, and lay out a dashboard for them. Prints the dashboard's JSON unless
told to write it.

options:
  --title TITLE   dashboard title (default: ephyra)
  --prefix NAME   metric prefix given to the sink (default: ephyra)
  --scope TAGS    only metrics with these tags, e.g. env:ci (default: *)
  --days N        look this far back for properties (default: 7)
  --print         print the dashboard's JSON (the default)
  --create        create the dashboard and print its id and path
  --update ID     replace the dashboard with this id
";

enum Write {
    Print,
    Create,
    Update(String),
}

struct Options {
    title: String,
    prefix: String,
    scope: String,
    days: u64,
    write: Write,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    if args.next().as_deref() != Some("dashboard") {
        return Err(USAGE.to_string());
    }
    let mut options = Options {
        title: "ephyra".to_string(),
        prefix: "ephyra".to_string(),
        scope: "*".to_string(),
        days: 7,
        write: Write::Print,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value\n\n{USAGE}"));
        match flag.as_str() {
            "--title" => options.title = value()?,
            "--prefix" => options.prefix = value()?,
            "--scope" => options.scope = value()?,
            "--days" => options.days = value()?.parse().map_err(|e| format!("--days: {e}"))?,
            "--print" => options.write = Write::Print,
            "--create" => options.write = Write::Create,
            "--update" => options.write = Write::Update(value()?),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    Ok(options)
}

/// The Datadog API, with the org's keys.
struct Api {
    agent: ureq::Agent,
    base: String,
    api_key: String,
    app_key: String,
}

impl Api {
    fn from_env() -> Result<Api, String> {
        let var = |name: &str| std::env::var(name).map_err(|_| format!("{name} is not set"));
        let site = std::env::var("DD_SITE").unwrap_or_else(|_| "datadoghq.com".to_string());
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Api {
            agent,
            base: format!("https://api.{site}"),
            api_key: var("DD_API_KEY")?,
            app_key: var("DD_APP_KEY")?,
        })
    }

    fn read(
        path: &str,
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Value, String> {
        let mut response = response.map_err(|e| format!("{path}: {e}"))?;
        let status = response.status();
        let body = response.body_mut().read_to_string().map_err(|e| format!("{path}: {e}"))?;
        if !status.is_success() {
            return Err(format!("{path}: {status}: {body}"));
        }
        serde_json::from_str(&body).map_err(|e| format!("{path}: {e}"))
    }

    fn query(&self, query: &str, from: u64, to: u64) -> Result<Value, String> {
        let path = "/api/v1/query";
        let response = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .query("from", from.to_string())
            .query("to", to.to_string())
            .query("query", query)
            .call();
        Api::read(path, response)
    }

    fn write(&self, path: &str, update: bool, body: &Value) -> Result<Value, String> {
        let url = format!("{}{path}", self.base);
        let request = if update { self.agent.put(url) } else { self.agent.post(url) };
        let response = request
            .header("DD-API-KEY", &self.api_key)
            .header("DD-APPLICATION-KEY", &self.app_key)
            .send_json(body);
        Api::read(path, response)
    }
}

fn run(options: Options) -> Result<(), String> {
    let api = Api::from_env()?;
    let to = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let from = to.saturating_sub(options.days * 86_400);
    let mut schema = Schema::default();
    for (kind, query) in discovery_queries(&options.prefix, &options.scope) {
        let result = api.query(&query, from, to)?;
        for series in result["series"].as_array().into_iter().flatten() {
            if let Some(scope) = series["scope"].as_str() {
                schema.add_series(kind, scope);
            }
        }
    }
    if schema.properties.is_empty() {
        return Err(format!(
            "no {}.* metrics matching {{{}}} in the last {} days",
            options.prefix, options.scope, options.days
        ));
    }
    eprintln!(
        "{} properties: {}",
        schema.properties.len(),
        schema.properties.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    let board = dashboard(&options.title, &options.prefix, &schema);
    let written = match &options.write {
        Write::Print => {
            println!("{}", serde_json::to_string_pretty(&board).expect("JSON values serialise"));
            return Ok(());
        },
        Write::Create => api.write("/api/v1/dashboard", false, &board)?,
        Write::Update(id) => api.write(&format!("/api/v1/dashboard/{id}"), true, &board)?,
    };
    // The API site does not say which subdomain the org's web app is on.
    let id = written["id"].as_str().unwrap_or("?");
    println!(
        "dashboard {id}: {} on the org's Datadog site",
        written["url"].as_str().unwrap_or_default()
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse(std::env::args().skip(1)).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        },
    }
}
