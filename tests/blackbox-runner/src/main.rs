mod runner;

fn main() {
    match runner::run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    }
}
