//! `hexa spend`: what inference has cost, from the local log.
//!
//! Every call hexa makes appends a line to `~/.hexa/inference-log.jsonl`
//! with tokens, source and, when the provider reported one, a dollar cost.
//! Local models report none. `claude -p` reports its own `total_cost_usd`.
//! This verb sums that file: today, the last seven days, and all of it, by
//! source and by model, and shows the daily budget if the project declares
//! one in `.hexa/project.json` under `inference.budget_usd_per_day`.

use clap::Args;
use colored::Colorize;
use serde_json::Value;
use std::collections::BTreeMap;

use hexa_infer::spend_report::{self as spend, Totals};

#[derive(Args, Debug)]
pub struct SpendArgs {
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

fn by_key(rows: &[&Value], key: &str) -> BTreeMap<String, Totals> {
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for r in rows {
        let k = r.get(key).and_then(Value::as_str).unwrap_or("unknown").to_string();
        out.entry(k).or_default().push((*r).clone());
    }
    out.into_iter().map(|(k, v)| (k, spend::totals(&v))).collect()
}

fn line(label: &str, t: &Totals) -> String {
    let cost = if t.priced_calls > 0 {
        format!("${:.2} over {} priced call{}", t.cost_usd, t.priced_calls, if t.priced_calls == 1 { "" } else { "s" })
    } else {
        "no priced calls".to_string()
    };
    format!("{:<10} {:>5} calls · {:>9} in · {:>8} out · {}", label, t.calls, t.input_tokens, t.output_tokens, cost)
}

pub async fn run(args: SpendArgs) -> anyhow::Result<()> {
    let rows = hexa_infer::spend_entries();
    let week = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    let today_rows = spend::since(&rows, &spend::start_of_today());
    let week_rows = spend::since(&rows, &week);
    let _all_rows: Vec<&Value> = rows.iter().collect();
    let today = spend::totals(&today_rows.iter().map(|r| (*r).clone()).collect::<Vec<_>>());
    let seven = spend::totals(&week_rows.iter().map(|r| (*r).clone()).collect::<Vec<_>>());
    let all = spend::totals(&rows);
    let budget = hexa_infer::spend_budget();

    if args.json {
        let t = |t: &Totals| serde_json::json!({ "calls": t.calls, "input_tokens": t.input_tokens, "output_tokens": t.output_tokens, "cost_usd": t.cost_usd, "priced_calls": t.priced_calls });
        let group = |m: BTreeMap<String, Totals>| -> Value { m.iter().map(|(k, v)| (k.clone(), t(v))).collect::<serde_json::Map<_, _>>().into() };
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "log": hexa_infer::spend_log_path().display().to_string(),
            "today": t(&today), "last_7_days": t(&seven), "all": t(&all),
            "by_source_7_days": group(by_key(&week_rows, "source")),
            "by_model_7_days": group(by_key(&week_rows, "model")),
            "budget_usd_per_day": budget,
            "explain": {
                "cost_usd": "sum of costs the provider reported; a local model reports none and is not priced",
                "priced_calls": "how many calls carried a cost",
                "budget": "inference.budget_usd_per_day in .hexa/project.json; the frontier path refuses once today's priced spend reaches it"
            }
        }))?);
        return Ok(());
    }

    println!("{} Inference spend ({})", "\u{2b21}".cyan(), hexa_infer::spend_log_path().display());
    if rows.is_empty() {
        println!("  nothing recorded yet");
        return Ok(());
    }
    println!();
    println!("  {}", line("today", &today));
    println!("  {}", line("7 days", &seven));
    println!("  {}", line("all", &all));
    println!();
    println!("  {}", "By source, last 7 days".bold());
    for (k, t) in by_key(&week_rows, "source") {
        println!("  {}", line(&k, &t));
    }
    println!();
    println!("  {}", "By model, last 7 days".bold());
    for (k, t) in by_key(&week_rows, "model") {
        println!("  {}", line(&k, &t));
    }
    println!();
    match budget {
        Some(b) => {
            let left = (b - today.cost_usd).max(0.0);
            println!("  budget  ${b:.2} per day · ${:.2} spent today · ${left:.2} left", today.cost_usd);
        }
        None => println!("  budget  none (set inference.budget_usd_per_day in .hexa/project.json)"),
    }
    println!("  {}", "A local model reports no cost and is not priced. A claude -p call reports its own.".dimmed());
    Ok(())
}
