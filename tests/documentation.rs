use cli_command_kit::Mode;

#[test]
fn agent_guide_covers_live_command_interface() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut text = String::new();
    for file in [
        "skills/fetch-track/SKILL.md",
        "skills/fetch-track/references/commands.md",
        "skills/fetch-track/references/progress.md",
        "skills/fetch-track/references/setup.md",
    ] {
        text.push_str(&std::fs::read_to_string(root.join(file)).unwrap());
        text.push('\n');
    }
    let command = fetch_track_cli::cli::command(Mode::Agent).unwrap();
    let missing =
        cli_command_kit::skill_omissions(&command, &fetch_track_cli::cli::help_options(), &text);
    assert!(
        missing.is_empty(),
        "undocumented CLI interface: {missing:?}"
    );
}
