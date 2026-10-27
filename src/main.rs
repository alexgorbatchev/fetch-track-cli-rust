use fetch_track_cli::{CancellationToken, cli};
#[tokio::main]
async fn main() -> std::process::ExitCode {
    let mode = cli_command_kit::Mode::from_env();
    let mut command = match cli::command(mode) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERR: {}", cli_command_kit::display_message(&e.to_string()));
            return std::process::ExitCode::FAILURE;
        }
    };
    let matches = match command.try_get_matches_from_mut(std::env::args_os()) {
        Ok(m) => m,
        Err(e) => {
            let code = e.exit_code() as u8;
            let _ = e.print();
            return code.into();
        }
    };
    let token = CancellationToken::new();
    let signal = token.clone();
    let listener = tokio::spawn(async move {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {r=tokio::signal::ctrl_c()=>r?,_=terminate.recv()=>{}}
        signal.cancel();
        std::io::Result::Ok(())
    });
    let result = cli::execute(&matches, mode, &token).await;
    listener.abort();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "ERR: {}",
                cli_command_kit::display_message(&error.to_string())
            );
            cli::exit_code(&error).into()
        }
    }
}
