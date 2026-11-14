//! Client-specific work that still reads or writes what core owns: Antigravity's
//! reader stages rows in the ledger, Claude's desktop questions live on
//! AgentHudCore, and WorkBuddy's readings walk core's log heads. Each moves to
//! its client's folder in `agents` once those seams have an interface
//! (docs/agent-contract.md).
pub(crate) mod antigravity;
pub(crate) mod claude_desktop;
pub(crate) mod claude_processes;
pub(crate) mod claude_project;
pub(crate) mod workbuddy;

impl crate::AgentHudCore {
    /// Whether the session list leaves this log's session out on purpose:
    /// gone, archived, internal, or a session a client shows elsewhere (a
    /// Claude project's worker belongs to its project's row). Shared code
    /// asks only this, never which client.
    pub(crate) fn hidden_from_list(&self, log: &crate::LogHead) -> bool {
        log.gone || log.archived || log.live.is_internal || self.is_claude_project_worker(log)
    }
}
