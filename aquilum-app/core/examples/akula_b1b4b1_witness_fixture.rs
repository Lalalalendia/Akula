//! Synthetic CI-only witness keypair fixture. Never a production key.
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: fixture SEED_FILE PUBLIC_OUTPUT".into());
    }
    let seed: [u8; 32] = fs::read(&args[1]).map_err(|e| e.to_string())?
        .try_into().map_err(|_| "seed must be 32 bytes")?;
    let signer = Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_| "invalid seed")?;
    fs::write(&args[2], signer.public_key().as_ref())
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("B1B4B1_FIXTURE_REFUSED: {e}");
        std::process::exit(2);
    }
}
