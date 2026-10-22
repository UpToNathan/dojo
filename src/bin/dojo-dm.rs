//! The Dojo display manager. See `dojo::dm::service`.

fn main() {
    // The display manager relays every password: no core dump, no same-user ptrace.
    dojo::lock::secret::protect_process();
    let arguments = std::env::args_os().skip(1).collect();
    let (settings, parts) = match dojo::dm::config::installed(arguments) {
        Ok(configured) => configured,
        Err(unusable) => {
            eprintln!("dojo-dm: {unusable}");
            std::process::exit(78);
        }
    };
    std::process::exit(dojo::dm::service::run(settings, parts));
}
