pub(crate) struct DurationPretty(pub std::time::Duration);

impl std::fmt::Display for DurationPretty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}s", self.0.as_secs())
    }
}
