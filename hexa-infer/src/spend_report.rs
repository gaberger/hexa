//! What the spend log says: sums, a day's rows, and whether the budget allows
//! another frontier call. Pure — the log itself is read by `crate::spend`.

use serde_json::Value;

/// Sums over a set of rows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Totals {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Dollars, over the rows that reported a cost.
    pub cost_usd: f64,
    /// How many rows reported a cost.
    pub priced_calls: u64,
}

pub fn totals(rows: &[Value]) -> Totals {
    let mut t = Totals::default();
    for r in rows {
        t.calls += 1;
        t.input_tokens += r.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        t.output_tokens += r.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        if let Some(c) = r.get("cost_usd").and_then(Value::as_f64) {
            t.cost_usd += c;
            t.priced_calls += 1;
        }
    }
    t
}

/// Rows whose `ts` is on or after `since` (RFC 3339 compares as text).
pub fn since<'a>(rows: &'a [Value], since: &str) -> Vec<&'a Value> {
    rows.iter()
        .filter(|r| r.get("ts").and_then(Value::as_str).map(|t| t >= since).unwrap_or(false))
        .collect()
}

/// Start of today, UTC, as RFC 3339.
pub fn start_of_today() -> String {
    chrono::Utc::now().format("%Y-%m-%dT00:00:00+00:00").to_string()
}

/// Dollars spent today over the rows that reported a cost.
fn today_cost_usd(rows: &[Value]) -> f64 {
    let today = since(rows, &start_of_today());
    today.iter().filter_map(|r| r.get("cost_usd").and_then(Value::as_f64)).sum()
}

/// Whether the frontier path may spend, given the log and the declared
/// budget. `Ok` when there is no budget or today's priced spend is under it;
/// `Err` names the numbers and the key.
pub fn budget_verdict(rows: &[Value], budget: Option<f64>) -> Result<(), String> {
    let Some(budget) = budget else {
        return Ok(());
    };
    let spent = today_cost_usd(rows);
    if spent >= budget {
        return Err(format!(
            "today's frontier spend ${spent:.2} has reached the budget ${budget:.2} (inference.budget_usd_per_day in .hexa/project.json)"
        ));
    }
    Ok(())
}
