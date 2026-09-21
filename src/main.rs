//! CLI entry point.

use std::io::Write;

use anyhow::Result;
use clap::Parser;
use fubo_account_generator::{Account, ProxyList, account, client, flow, proxy};

#[derive(Parser)]
#[command(name = "fubo-gen", about = "Create fubo.tv accounts via raw requests")]
struct Args {
    /// Number of accounts to create.
    #[arg(default_value_t = 1)]
    count: usize,
    /// Fixed shared password (random per account when omitted).
    #[arg(long)]
    password: Option<String>,
    /// JSONL output file (appended).
    #[arg(long, default_value = "accounts.jsonl")]
    json_out: std::path::PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();

    let domain = std::env::var(account::CATCHALL_DOMAIN_ENV).unwrap_or_default();
    anyhow::ensure!(
        !domain.trim().is_empty(),
        "{} must be set (e.g. in .env)",
        account::CATCHALL_DOMAIN_ENV
    );
    let domain = domain.trim();

    let proxies_path = std::env::var(proxy::PROXIES_FILE_ENV)
        .unwrap_or_else(|_| String::from(proxy::PROXIES_FILE_DEFAULT));
    let proxies = ProxyList::load(std::path::Path::new(&proxies_path))?;
    if proxies.is_empty() {
        eprintln!("no proxies loaded from {proxies_path}");
    } else {
        eprintln!("loaded {} proxies from {proxies_path}", proxies.len());
    }

    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.json_out)?;
    let mut out = std::io::BufWriter::new(out);

    let mut failures = 0usize;
    for index in 0..args.count {
        let mut account = match &args.password {
            Some(password) => Account::with_password(password.clone(), domain),
            None => Account::generate(domain),
        };

        let client = client::build_client(proxies.random())?;

        match flow::create_account(&client, &mut account).await {
            Ok(created) => {
                println!(
                    "{}  {}  {}",
                    created.email, created.password, created.user_id
                );
                writeln!(out, "{}", serde_json::to_string(&created)?)?;
            }
            Err(error) => {
                failures += 1;
                eprintln!("[{}/{}] failed: {error:#}", index + 1, args.count);
            }
        }
    }

    if failures == args.count {
        anyhow::bail!("all {failures} account(s) failed");
    }
    Ok(())
}
