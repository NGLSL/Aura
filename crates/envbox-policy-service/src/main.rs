fn main() {
    #[cfg(windows)]
    if let Err(error) = envbox_policy_service::windows_service::dispatch() {
        eprintln!("AuraPolicyService: {error}");
        std::process::exit(1);
    }
}
