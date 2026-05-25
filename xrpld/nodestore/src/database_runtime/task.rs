pub trait Task: Send + Sync + 'static {
    fn perform_scheduled_task(&self);
}
