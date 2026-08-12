pub trait Task: Send + Sync + 'static {
    fn perform_scheduled_task(&self);

    /// Extra heap ownership retained only because this task is queued or
    /// running. Scheduler adds the concrete task object size itself; task
    /// implementations expose only ownership they directly encapsulate.
    fn retained_bytes(&self) -> usize {
        0
    }
}
