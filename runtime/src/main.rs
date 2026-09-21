use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments (--config <path>, -c <path>, or positional path)
    let args: Vec<String> = std::env::args().collect();
    let mut config_arg = None;
    let mut i = 1;
    while i < args.len() {
        if (args[i] == "--config" || args[i] == "-c") && i + 1 < args.len() {
            config_arg = Some(args[i + 1].clone());
            i += 2;
        } else if !args[i].starts_with('-') && config_arg.is_none() {
            config_arg = Some(args[i].clone());
            i += 1;
        } else {
            i += 1;
        }
    }

    spectra_flux::runner::start_server(config_arg).await
}
