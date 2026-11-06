//! `castor-cli` — development binary. Shipped builds run the same code as `castor cli …`.

fn main() {
    std::process::exit(castor_cli::run(std::env::args_os()));
}
