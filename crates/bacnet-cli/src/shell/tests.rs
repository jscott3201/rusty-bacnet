use super::*;

#[test]
fn tokenize_simple() {
    let tokens = tokenize("read 192.168.1.10 ai:1 pv");
    assert_eq!(tokens, vec!["read", "192.168.1.10", "ai:1", "pv"]);
}

#[test]
fn tokenize_quoted_string() {
    let tokens = tokenize("write 10.0.1.5 av:1 pv \"hello world\"");
    assert_eq!(
        tokens,
        vec!["write", "10.0.1.5", "av:1", "pv", "\"hello world\""]
    );
}

#[test]
fn tokenize_empty() {
    let tokens = tokenize("");
    assert!(tokens.is_empty());
}

#[test]
fn tokenize_extra_whitespace() {
    let tokens = tokenize("  read   10.0.1.5   ai:1   pv  ");
    assert_eq!(tokens, vec!["read", "10.0.1.5", "ai:1", "pv"]);
}

#[test]
fn shell_helper_completions() {
    let helper = ShellHelper::new();
    assert!(!helper.commands.is_empty());
    assert!(!helper.object_types.is_empty());
    assert!(!helper.properties.is_empty());
}
