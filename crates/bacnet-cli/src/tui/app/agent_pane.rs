//! The agent pane's state (#2185; design §9): screen 8, while the
//! session's MCP listener runs. It lists the attached agents' tool calls,
//! newest first, from the listener's [`Board`](bacnet_mcp::attach::Board):
//! the sessions record each call there on their own runtime, and every
//! tick takes a copy when it changed, without waiting for the lock, so a
//! busy session never holds up a frame. The ring keeps the latest
//! [`CALLS`](bacnet_mcp::attach::CALLS), dropping the oldest, counted.
//!
//! The pane also tells the board whether the write session here is armed,
//! which `bacnet://session` reports to the agents, read-only.

use super::App;
use super::devices::Movement;

/// The pane: the board and the copy of its calls, the first row shown,
/// and the arming last seen.
#[derive(Debug, Default)]
pub(crate) struct AgentPane {
    /// The listener's board, once it listens.
    #[cfg(feature = "mcp")]
    pub(crate) board: Option<bacnet_mcp::attach::Board>,
    /// The calls, newest first, as last copied.
    #[cfg(feature = "mcp")]
    pub(crate) calls: bacnet_mcp::attach::Calls,
    /// The first call row shown, counted from the newest; the view clamps
    /// it. Scrolled down, it moves with the rows each new call pushes in,
    /// so the same calls stay in view.
    pub(crate) scroll: std::cell::Cell<usize>,
    /// Whether the write session was armed at the last look.
    armed: bool,
}

impl AgentPane {
    /// The write session is `armed` now: true when that changed since the
    /// last look. The board hears it either way.
    pub(super) fn armed_now(&mut self, armed: bool) -> bool {
        #[cfg(feature = "mcp")]
        if let Some(board) = &self.board {
            board.set_armed(armed);
        }
        std::mem::replace(&mut self.armed, armed) != armed
    }

    /// Scroll the call list.
    pub(super) fn scroll(&self, movement: Movement) {
        let at = self.scroll.get();
        self.scroll.set(match movement {
            Movement::Up => at.saturating_sub(1),
            Movement::Down => at.saturating_add(1),
            Movement::PageUp => at.saturating_sub(10),
            Movement::PageDown => at.saturating_add(10),
            Movement::Home => 0,
            Movement::End => usize::MAX,
        });
    }
}

impl App {
    /// The listener's `board` arrived: the pane reads from it, and it hears
    /// the arming here.
    #[cfg(feature = "mcp")]
    pub(super) fn agents_board(&mut self, board: bacnet_mcp::attach::Board) {
        board.set_armed(self.safety.armed.is_some());
        let pane = &mut self.safety.agents.pane;
        pane.calls = bacnet_mcp::attach::Calls::default();
        pane.board = Some(board);
        self.dirty = true;
    }

    /// Each tick: copy the calls when they changed, and redraw the pane
    /// when it is shown, its waiting requests' ages included.
    pub(super) fn agent_pane_tick(&mut self) {
        #[cfg(feature = "mcp")]
        {
            let pane = &mut self.safety.agents.pane;
            if let Some(calls) = pane
                .board
                .as_ref()
                .and_then(|b| b.calls_since(pane.calls.seq))
            {
                let scroll = pane.scroll.get();
                if scroll > 0 {
                    let added = usize::try_from(calls.seq.saturating_sub(pane.calls.seq))
                        .unwrap_or(usize::MAX);
                    pane.scroll.set(scroll.saturating_add(added));
                }
                pane.calls = calls;
                self.dirty |= self.screen == super::watch::Screen::Agents;
            }
        }
    }

    /// `8`: the agent pane, while the session's MCP listener runs.
    pub(super) fn show_agents(&mut self) {
        if self.mcp.is_some() {
            self.screen = super::watch::Screen::Agents;
        } else {
            self.flash(
                "No MCP listener in this session: agents attach to a profile authorized for MCP.",
                super::FLASH_FOR,
            );
        }
    }
}
