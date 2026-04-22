pub trait Metrics: Send + Sync {
    fn increase_peer_count(&self);
    fn decrease_peer_count(&self);
    fn increase_gossipsub_message_count(&self);
    fn increase_invalid_gossipsub_count(&self);
    fn increase_connection_failure_count(&self);
}

impl Metrics for Box<dyn Metrics> {
    fn increase_peer_count(&self) {
        self.as_ref().increase_peer_count();
    }

    fn decrease_peer_count(&self) {
        self.as_ref().increase_peer_count();
    }

    fn increase_gossipsub_message_count(&self) {
        self.as_ref().increase_gossipsub_message_count();
    }

    fn increase_invalid_gossipsub_count(&self) {
        self.as_ref().increase_invalid_gossipsub_count();
    }

    fn increase_connection_failure_count(&self) {
        self.as_ref().increase_connection_failure_count();
    }
}
