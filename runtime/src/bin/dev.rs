use anyhow::Result;
use std::path::Path;

fn find_local_dev_config() -> Option<String> {
    // 1. Check generic local config files first
    let generic_candidates = [
        "spectra-flux.local.toml",
        "spectra-flux-local.toml",
        "../spectra-flux.local.toml",
        "../spectra-flux-local.toml",
    ];
    for candidate in generic_candidates {
        if Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }

    // 2. Dynamically scan current and parent directory for any gitignored local config file
    //    matching pattern: spectra-flux.*local*.toml
    for search_dir in [".", ".."] {
        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with("spectra-flux.")
                    && file_name.contains("local")
                    && file_name.ends_with(".toml")
                {
                    return Some(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }

    // 3. Fallback to standard base configuration
    for base in ["spectra-flux.toml", "../spectra-flux.toml"] {
        if Path::new(base).exists() {
            return Some(base.to_string());
        }
    }

    None
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Ensure local development data and fluxcells storage volume directories exist
    let _ = std::fs::create_dir_all("./data/fluxcells");
    let _ = std::fs::create_dir_all("data/fluxcells");

    // 2. Set developer-friendly default log levels if unset
    if std::env::var("RUST_LOG").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "info,spectra_flux=debug");
        }
    }

    // 3. Parse CLI args or auto-detect local dev config file
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

    // Auto-detect local config if no explicit argument or FLUX_CONFIG environment variable was provided
    let has_env_config = std::env::var("FLUX_CONFIG").is_ok();

    if config_arg.is_none() && !has_env_config {
        config_arg = find_local_dev_config();
    }

    println!("🛠️  [SpectraFlux Dev] Booting in local development mode...");
    if let Some(ref c) = config_arg {
        println!("🛠️  [SpectraFlux Dev] Active config: {}", c);
    }
    println!("🛠️  [SpectraFlux Dev] Persistent volume root: ./data/fluxcells");

    spectra_flux::runner::start_server(config_arg).await
}
