//! Interactive REPL for the BTL test harness.

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use bacnet_btl::engine::registry::TestRegistry;
use bacnet_btl::engine::runner::{RunConfig, TestRunner};
use bacnet_btl::engine::selector::TestFilter;
use bacnet_btl::report::terminal;
use bacnet_btl::self_test::in_process::InProcessServer;
use bacnet_btl::tests;

pub async fn run_shell() {
    println!("bacnet-test shell — interactive BTL test REPL");
    println!("Type 'help' for commands, 'exit' to quit.\n");

    let mut rl = DefaultEditor::new().expect("Failed to create editor");

    loop {
        match rl.readline("bacnet-test> ") {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(line);

                let parts: Vec<&str> = line.split_whitespace().collect();
                match parts[0] {
                    "help" => print_help(),
                    "exit" | "quit" => break,
                    "list" => cmd_list(&parts[1..]),
                    "self-test" => cmd_self_test(&parts[1..]).await,
                    _ => {
                        println!("Unknown command: '{}'. Type 'help' for commands.", parts[0]);
                    }
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }
}

fn print_help() {
    println!("Commands:");
    println!("  list [--section N] [--tag TAG]   List available tests");
    println!("  self-test [--section N] [--tag TAG]  Run self-test");
    println!("  help                             Show this help");
    println!("  exit                             Exit the shell");
}

fn cmd_list(args: &[&str]) {
    let mut registry = TestRegistry::new();
    tests::register_all(&mut registry);

    let mut section = None;
    let mut tag = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--section" if i + 1 < args.len() => {
                section = Some(args[i + 1].to_string());
                i += 2;
            }
            "--tag" if i + 1 < args.len() => {
                tag = Some(args[i + 1].to_string());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let filter = TestFilter {
        section,
        tag,
        ..Default::default()
    };

    let caps = bacnet_btl::iut::capabilities::IutCapabilities::default();
    let selected = bacnet_btl::engine::selector::TestSelector::select(&registry, &caps, &filter);

    if selected.is_empty() {
        println!("No tests match the given filters.");
        return;
    }

    for test in &selected {
        println!("  {:<8} {}", test.id, test.name);
    }
    println!("  {} tests", selected.len());
}

async fn cmd_self_test(args: &[&str]) {
    let mut section = None;
    let mut tag = None;
    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--section" if i + 1 < args.len() => {
                section = Some(args[i + 1].to_string());
                i += 2;
            }
            "--tag" if i + 1 < args.len() => {
                tag = Some(args[i + 1].to_string());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let server = match InProcessServer::start().await {
        Ok(s) => s,
        Err(e) => {
            println!("Failed to start server: {e}");
            return;
        }
    };

    let mut ctx = match server.build_context().await {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to build context: {e}");
            return;
        }
    };

    let mut registry = TestRegistry::new();
    tests::register_all(&mut registry);
    let runner = TestRunner::new(registry);

    let config = RunConfig {
        filter: TestFilter {
            section,
            tag,
            ..Default::default()
        },
        ..Default::default()
    };

    let run = runner.run(&mut ctx, &config).await;
    terminal::print_test_run(&run, false);
}
