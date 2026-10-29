//! The arguments of `bacnet mcp` (#2066), out of `args.rs` for its size cap.

use clap::Subcommand;

/// What `bacnet mcp` does.
#[derive(Debug, Subcommand)]
pub(crate) enum McpAction {
    /// Serve MCP on stdin and stdout until the agent host closes them.
    /// Stdout carries only JSON-RPC; logs go to stderr.
    Serve {
        /// Serve on stdin and stdout, the transport an agent host launches.
        #[arg(long, required = true)]
        stdio: bool,
        /// Named profile to connect with; its mcp table limits what the
        /// agent may reach. Needed unless --passive; no transport flags with it.
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,
        /// Build no client and send nothing: only the tools that need no
        /// network are offered.
        #[arg(long)]
        passive: bool,
    },
    /// Record, after showing what an agent may reach, that you authorize
    /// the MCP server on a profile. Needs a terminal.
    Authorize {
        /// The profile to authorize.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },
}
