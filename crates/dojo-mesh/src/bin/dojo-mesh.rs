fn main() -> std::process::ExitCode {
    dojo_mesh::cli::main(std::env::args().skip(1).collect())
}
