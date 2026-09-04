use ygopro::cli::*;

#[tokio::main]
async fn main() {
    env_logger::init();
    ygopro::init();
    let args = std::env::args().collect::<Vec<String>>();
    let (port, hostinfo, replay_mode, pre_seeds) = match parse_cli_args(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            log::error!("{error}");
            std::process::exit(1);
        }
    };
    let duel = build_duel_host(hostinfo, replay_mode, pre_seeds);
    start_local_server(port, duel).await;
}
