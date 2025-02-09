use anyhow::{Context, Result};
use ledger::{actions::AccountAction, database::Database};
use std::{fs::File, io::BufReader};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: {} <input.csv>", args[0]);
        std::process::exit(1);
    }
    let path = &args[1];
    let reader = BufReader::new(File::open(path).context("failed to open example file")?);
    let mut reader = csv::ReaderBuilder::new()
        // we have headers in the CSV
        .has_headers(true)
        // allow for comments in the CSV using #
        .comment(Some(b'#'))
        // dispute, resolve, and chargeback actions don't have an amount field
        .flexible(true)
        // allow for whitespaces in the CSV
        .trim(csv::Trim::All)
        .from_reader(reader);

    let mut db = Database::new();
    for (n, record) in reader.deserialize::<AccountAction>().enumerate() {
        match record {
            Err(e) => {
                eprintln!("failed to deserialize record {n}: {e}");
            }
            Ok(action) => {
                if let Err(e) = db.perform_action(action) {
                    eprintln!("failed to perform action {n}: {e}");
                }
            }
        }
    }
    let mut wtr = csv::Writer::from_writer(std::io::stdout());
    for client in db.clients() {
        wtr.serialize(client)
            .context("failed to serialize client")?;
    }
    Ok(())
}
