//! The UI loop: one `select!` over terminal input, a frame interval, the
//! worker's events and signals. It runs on the thread that `block_on`s the
//! CLI's boxed future, not in `tokio::spawn`, so a slow `terminal.draw` never
//! holds up a worker task.
//!
//! Only key presses count (Windows also reports releases). Redraws happen
//! when the state is dirty: straight after a key, so typing feels immediate,
//! and otherwise at most once per frame, so a burst of worker events costs one
//! draw per frame however many events it holds.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use ratatui::backend::Backend;
use ratatui::crossterm::event::{Event, KeyEventKind};
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use super::app::{update, Action, App};
use super::keymap::map_key;
use super::message::{Command, WorkerEvent};
use super::terminal::Signals;
use super::view;

/// Worker events applied per wake-up before yielding to input and frames.
const DRAIN_BUDGET: usize = 256;

/// Exit message when a panic elsewhere has already restored the terminal.
pub(crate) const INTERNAL_ERROR: &str = "internal error (see the panic message above)";

/// Exit message when the worker task ends while the UI is still running.
pub(crate) const WORKER_STOPPED: &str = "internal error: the network worker stopped unexpectedly";

/// Channels and settings the loop runs with.
pub(crate) struct LoopIo<I> {
    /// Terminal events.
    pub(crate) input: I,
    /// Events from the worker.
    pub(crate) events: mpsc::Receiver<WorkerEvent>,
    /// Commands to the worker.
    pub(crate) commands: mpsc::Sender<Command>,
    /// The worker's dropped-events counter.
    pub(crate) dropped: Arc<AtomicU64>,
    /// Frames per second.
    pub(crate) fps: u16,
    /// Signals that end the loop.
    pub(crate) signals: Signals,
    /// False once the terminal has been restored behind the loop's back (by
    /// the panic hook). [`terminal::is_active`](super::terminal::is_active)
    /// in production.
    pub(crate) alive: fn() -> bool,
}

/// Counters for tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LoopStats {
    /// Frames drawn.
    pub(crate) frames: u64,
}

/// Run until the app quits, input ends, or a signal arrives.
pub(crate) async fn run<B, I>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    io: &mut LoopIo<I>,
) -> Result<LoopStats, Box<dyn std::error::Error>>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
    I: Stream<Item = io::Result<Event>> + Unpin,
{
    let mut stats = LoopStats::default();
    let period = Duration::from_secs(1) / u32::from(io.fps.max(1));
    let mut frames = tokio::time::interval(period);
    frames.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut worker_open = true;
    let alive = io.alive;
    draw(terminal, app, &mut stats, alive)?;
    while !app.quit {
        if !alive() {
            app.fail(INTERNAL_ERROR, 1);
            break;
        }
        tokio::select! {
            input = io.input.next() => match input {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    if let Some(action) = map_key(app, key) {
                        apply(app, action, &io.commands);
                    }
                    if app.dirty && !app.quit {
                        draw(terminal, app, &mut stats, alive)?;
                    }
                }
                Some(Ok(Event::Resize(..))) => {
                    apply(app, Action::Resize, &io.commands);
                    draw(terminal, app, &mut stats, alive)?;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.into()),
                None => app.quit = true,
            },
            _ = frames.tick() => {
                let tick = Action::Tick {
                    now: tokio::time::Instant::now().into_std(),
                    dropped: io.dropped.load(Ordering::Relaxed),
                    log_generation: app.log.generation(),
                };
                apply(app, tick, &io.commands);
                if app.dirty {
                    draw(terminal, app, &mut stats, alive)?;
                }
            }
            event = io.events.recv(), if worker_open => match event {
                Some(event) => {
                    apply(app, Action::Worker(event), &io.commands);
                    for _ in 1..DRAIN_BUDGET {
                        match io.events.try_recv() {
                            Ok(event) => apply(app, Action::Worker(event), &io.commands),
                            Err(_) => break,
                        }
                    }
                }
                None => {
                    // The worker owns the client; without it the screen
                    // would only go stale. It ends early only on a bug.
                    worker_open = false;
                    if !app.quit {
                        app.fail(WORKER_STOPPED, 1);
                    }
                }
            },
            signal = io.signals.recv() => {
                app.fail(format!("terminated by {}", signal.name), signal.code);
            }
        }
    }
    Ok(stats)
}

fn apply(app: &mut App, action: Action, commands: &mpsc::Sender<Command>) {
    let prior_link = app.link.clone();
    for command in update(app, action) {
        if let Err(error) = commands.try_send(command) {
            let message =
                format!("Command not admitted: {error}; retry after the worker catches up.");
            tracing::warn!("{message}");
            app.command_rejected(error.into_inner(), prior_link.clone(), message);
        }
    }
}

fn draw<B>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    stats: &mut LoopStats,
    alive: fn() -> bool,
) -> Result<(), Box<dyn std::error::Error>>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
    if !alive() {
        // A panic elsewhere restored the terminal: drawing now would paint
        // the alternate screen's frame over the shell in cooked mode.
        app.fail(INTERNAL_ERROR, 1);
        return Ok(());
    }
    app.settle();
    terminal.draw(|frame| view::draw(frame, app))?;
    app.dirty = false;
    stats.frames += 1;
    Ok(())
}
