use std::path::PathBuf;

use clap::{Parser, Subcommand};
use flovyn_worker_sdk::agent::session::{Session, SessionStatus};

#[allow(dead_code)] // Tools infrastructure for future ReactAgent integration
mod tools;

/// CLI for running and managing local Flovyn agents.
#[derive(Parser)]
#[command(name = "flovyn-agent", about = "Local Flovyn agent runner")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new local agent session
    Run {
        /// Agent kind (e.g., "coder", "analyzer")
        #[arg(long)]
        kind: String,

        /// Workspace directory the agent operates in
        #[arg(long, default_value = ".")]
        workspace: PathBuf,

        /// Queue name override. If not set, auto-generates from
        /// user.{username}.{hostname}.{workspace_dir_name}
        #[arg(long)]
        queue: Option<String>,
    },

    /// Resume a suspended agent session
    Resume {
        /// Session ID to resume (omit to resume most recent for workspace)
        session_id: Option<String>,

        /// Resume most recent session for this workspace
        #[arg(long)]
        workspace: Option<PathBuf>,
    },

    /// List agent sessions
    Sessions {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            kind,
            workspace,
            queue,
        } => cmd_run(&kind, &workspace, queue.as_deref()).await?,
        Commands::Resume {
            session_id,
            workspace,
        } => cmd_resume(session_id.as_deref(), workspace.as_deref()).await?,
        Commands::Sessions { status } => cmd_sessions(status.as_deref()).await?,
    }

    Ok(())
}

async fn cmd_run(
    kind: &str,
    workspace: &std::path::Path,
    queue_override: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use flovyn_worker_sdk::agent::queue::generate_queue_name;

    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());

    // Resolve queue name: explicit override or auto-generate
    let queue = match queue_override {
        Some(q) => q.to_string(),
        None => generate_queue_name(&workspace),
    };

    let (session, _storage) = Session::create(kind, &workspace).await?;

    println!("Created session: {}", session.session_id);
    println!("  Kind:      {}", session.agent_kind);
    println!("  Workspace: {}", session.workspace.display());
    println!("  Queue:     {}", queue);
    println!("  Database:  {}", session.db_path.display());
    println!();
    println!("Session is ready. Agent execution requires an AgentDefinition to be registered.");
    println!(
        "Use `flovyn-agent resume {}` to resume later.",
        session.session_id
    );

    Ok(())
}

async fn cmd_resume(
    session_id: Option<&str>,
    workspace: Option<&std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (session, _storage) = match (session_id, workspace) {
        (Some(id), _) => Session::resume(id).await?,
        (None, Some(ws)) => {
            let ws = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
            let sessions = Session::list().await?;
            let matching = sessions
                .into_iter()
                .find(|s| s.workspace == ws && s.status != SessionStatus::Completed);
            match matching {
                Some(s) => {
                    let id = s.session_id.clone();
                    Session::resume(&id).await?
                }
                None => {
                    eprintln!("No resumable session found for workspace: {}", ws.display());
                    std::process::exit(1);
                }
            }
        }
        (None, None) => {
            eprintln!("Provide a session ID or --workspace to resume");
            std::process::exit(1);
        }
    };

    println!("Resumed session: {}", session.session_id);
    println!("  Kind:      {}", session.agent_kind);
    println!("  Workspace: {}", session.workspace.display());
    println!("  Status:    {}", session.status);
    println!();
    println!("Agent execution requires an AgentDefinition to be registered.");

    Ok(())
}

async fn cmd_sessions(status_filter: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let sessions = Session::list().await?;

    let sessions: Vec<_> = if let Some(filter) = status_filter {
        sessions
            .into_iter()
            .filter(|s| s.status.to_string() == filter)
            .collect()
    } else {
        sessions
    };

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    // Print table header
    println!(
        "{:<38} {:<12} {:<30} {:<12} Last Active",
        "ID", "Kind", "Workspace", "Status"
    );
    println!("{}", "-".repeat(110));

    for session in &sessions {
        let workspace = session.workspace.display().to_string();
        let workspace_display = if workspace.len() > 28 {
            format!("...{}", &workspace[workspace.len() - 25..])
        } else {
            workspace
        };

        let elapsed = chrono::Utc::now() - session.last_activity;
        let ago = format_duration(elapsed);

        println!(
            "{:<38} {:<12} {:<30} {:<12} {}",
            session.session_id, session.agent_kind, workspace_display, session.status, ago,
        );
    }

    println!();
    println!("{} session(s) total", sessions.len());

    Ok(())
}

fn format_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
