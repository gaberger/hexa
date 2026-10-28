//! `hexa design` and `hexa drift` (ADR-2610271736): the Elephant-Goldfish
//! Model (EGM, <https://github.com/google/elephant-goldfish-model>, Google LLC,
//! Apache-2.0), on its prompts vendored and pinned under `assets/egm/`. The
//! work is `hexa_exec::egm`; this verb reads the arguments and the embedded
//! prompts, and prints.

use clap::{Args, Subcommand, ValueEnum};
use colored::Colorize;
use hexa_exec::egm::{self, Egm, ReviewScope, Workspace};

use crate::assets::Assets;

#[derive(Subcommand)]
pub enum DesignAction {
    /// Turn an idea into concepts_brief.md (the elephant, on EGM's egm-brainstorm),
    /// then have the critic goldfish review the brief
    Brainstorm {
        /// The idea, in plain words
        idea: String,
        /// The elephant's model; defaults to the t3 tier, else claude-text
        #[arg(long)]
        model: Option<String>,
        /// The critic goldfish's model; defaults to the t2.5 tier, else claude-text
        #[arg(long)]
        goldfish_model: Option<String>,
        /// Replace an existing concepts_brief.md
        #[arg(long)]
        force: bool,
    },
    /// Write design_draft.md from a brief (the elephant, on EGM's egm-draft)
    Draft {
        /// The brief; defaults to concepts_brief.md in the artifact directory
        brief: Option<String>,
        /// The elephant's model; defaults to the t3 tier, else claude-text
        #[arg(long)]
        model: Option<String>,
        /// Replace an existing design_draft.md
        #[arg(long)]
        force: bool,
    },
    /// Review a draft with EGM's goldfish reviewers, stage by stage (5 to 9).
    /// Writes goldfish_review_report.md; never edits the draft
    Review {
        /// The draft to review
        draft: String,
        /// Run this one stage (5, 6, 7, 8 or 9)
        #[arg(long)]
        stage: Option<u8>,
        /// Which stages to run when --stage is not given
        #[arg(long, value_enum, default_value = "full")]
        preset: PresetArg,
        /// The goldfish's model; defaults to the t2.5 tier, else claude-text
        #[arg(long)]
        model: Option<String>,
    },
    /// Revise the draft a review report names, applying only the findings you accept
    Resolve {
        /// The review report
        report: String,
        /// Finding ids to apply, e.g. S6-1,S7-2
        #[arg(long, value_delimiter = ',', required = true)]
        accept: Vec<String>,
        /// The elephant's model; defaults to the t3 tier, else claude-text
        #[arg(long)]
        model: Option<String>,
    },
    /// Turn a reviewed, committed design into a proposed ADR that cites it
    /// (ADR-2610281011). Refuses a draft that is untracked, uncommitted, or
    /// changed since its last review covering stages 5 to 8
    Adr {
        /// The design draft
        draft: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum PresetArg {
    Full,
    FeatureSpec,
    ImplementationPlan,
}

impl PresetArg {
    fn scope(self) -> ReviewScope {
        match self {
            PresetArg::Full => ReviewScope::Full,
            PresetArg::FeatureSpec => ReviewScope::FeatureSpecOnly,
            PresetArg::ImplementationPlan => ReviewScope::ImplementationPlanOnly,
        }
    }
}

#[derive(Args)]
pub struct DriftArgs {
    /// Check every Markdown document under this directory (ADRs excepted);
    /// default: README.md, ARCHITECTURE.md and docs/*.md
    pub dir: Option<String>,
    /// The goldfish's model; defaults to the t2.5 tier, else claude-text
    #[arg(long)]
    pub model: Option<String>,
    /// Check only the documents that changed since this git ref, or that
    /// name a path that did
    #[arg(long)]
    pub since: Option<String>,
    /// Exit non-zero when drift is found
    #[arg(long)]
    pub fail_on_drift: bool,
}

pub async fn run(action: DesignAction) -> anyhow::Result<()> {
    let egm = vendored()?;
    let ws = Workspace::at(hexa_infer::project_root()).map_err(anyhow::Error::msg)?;
    let say = |line: &str| println!("  {} {line}", "\u{25b8}".cyan());
    let title = |verb: &str| println!("{}", format!("\u{2b21} design {verb}").cyan().bold());
    let r = match action {
        DesignAction::Brainstorm { idea, model, goldfish_model, force } => {
            let elephant = egm::elephant_model(model.as_deref()).map_err(anyhow::Error::msg)?;
            let critic = egm::goldfish_model(goldfish_model.as_deref()).map_err(anyhow::Error::msg)?;
            title("brainstorm");
            egm::brainstorm(&egm, &ws, &idea, &elephant, &critic, force, &say).await.map(|()| {
                println!("\n  Read the brief and its critique, edit the brief, then `hexa design draft`.");
            })
        }
        DesignAction::Draft { brief, model, force } => {
            let elephant = egm::elephant_model(model.as_deref()).map_err(anyhow::Error::msg)?;
            title("draft");
            egm::draft(&egm, &ws, brief.as_deref(), &elephant, force, &say).await.map(|d| {
                println!("\n  Edit the draft, then `hexa design review {d}`.");
            })
        }
        DesignAction::Review { draft, stage, preset, model } => {
            let model = egm::goldfish_model(model.as_deref()).map_err(anyhow::Error::msg)?;
            let scope = stage.map(ReviewScope::Stage).unwrap_or(preset.scope());
            title("review");
            egm::review(&egm, &ws, &draft, scope, &model, &say).await.map(|r| {
                println!("\n  Decide which findings to take, then `hexa design resolve {r} --accept <ids>`.");
            })
        }
        DesignAction::Resolve { report, accept, model } => {
            let elephant = egm::elephant_model(model.as_deref()).map_err(anyhow::Error::msg)?;
            title("resolve");
            egm::resolve(&egm, &ws, &report, &accept, &elephant, &say).await.map(|_| {
                println!(
                    "\n  Read the revision. A person settles a design by committing it; it enters the pipeline as an ADR."
                );
            })
        }
        DesignAction::Adr { draft } => {
            title("adr");
            let now = chrono::Local::now().naive_local();
            match egm::design_adr(&ws, &draft, now, &say) {
                Ok((id, _)) => {
                    let cwd = std::env::current_dir()?;
                    super::loop_cmd::ensure_state_dir(&cwd)?;
                    super::loop_cmd::update_loop(&cwd, serde_json::json!({ "adr": id, "stage": "decide" }))
                        .map_err(anyhow::Error::msg)?;
                    println!("  {} the loop is under {id}, stage decide", "\u{2713}".green());
                    println!(
                        "\n  The ADR is Proposed. Edit its decisions, name its ports and gate, then `hexa spec draft {id}`."
                    );
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
    };
    r.map_err(anyhow::Error::msg)
}

pub async fn run_drift(args: DriftArgs) -> anyhow::Result<()> {
    let egm = vendored()?;
    let model = egm::goldfish_model(args.model.as_deref()).map_err(anyhow::Error::msg)?;
    let today = std::env::var("HEXA_TODAY")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    println!("{}", "\u{2b21} drift".cyan().bold());
    let say = |line: &str| println!("  {} {line}", "\u{25b8}".cyan());
    let root = hexa_infer::project_root();
    let (_, results) = egm::drift(&egm, &root, args.dir.as_deref(), args.since.as_deref(), &model, &today, &say)
        .await
        .map_err(anyhow::Error::msg)?;
    let drifted = results.iter().filter(|d| !d.divergences.is_empty()).count();
    for d in &results {
        if d.divergences.is_empty() {
            println!("  {} {}: no drift", "\u{2713}".green(), d.document);
        } else {
            println!("  {} {}: {} divergence(s)", "\u{2717}".red(), d.document, d.divergences.len());
        }
    }
    if args.fail_on_drift && drifted > 0 {
        anyhow::bail!("drift found in {drifted} document(s)");
    }
    Ok(())
}

/// The embedded EGM, verified against its pin (§1).
fn vendored() -> anyhow::Result<Egm> {
    let upstream = Assets::get_str("egm/UPSTREAM.json").ok_or_else(|| anyhow::anyhow!("assets/egm/UPSTREAM.json is not embedded"))?;
    let files = Assets::iter()
        .filter_map(|p| {
            let rel = p.strip_prefix("egm/")?.to_string();
            (rel != "UPSTREAM.json").then(|| Assets::get(&p).map(|f| (rel, f.data.into_owned())))?
        })
        .collect();
    Egm::verified(&upstream, files).map_err(anyhow::Error::msg)
}
