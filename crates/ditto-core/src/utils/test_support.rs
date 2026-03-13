pub fn should_skip_httpmock() -> bool {
    if can_bind_localhost() {
        return false;
    }
    eprintln!("skipping httpmock test: sandbox forbids binding to localhost");
    true
}

fn can_bind_localhost() -> bool {
    match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => {
            drop(listener);
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(err) => panic!("failed to bind localhost for httpmock tests: {err}"),
    }
}
