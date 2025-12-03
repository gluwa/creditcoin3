#[rstest::fixture]
pub fn logs() {
    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::DEBUG.into())
        .from_env_lossy();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_test_writer()
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .try_init();
}
