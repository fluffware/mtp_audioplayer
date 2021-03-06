use crate::actions::action::{Action, ActionFuture};
use crate::clip_queue::ClipQueue;
use crate::sample_buffer::SampleBuffer;
use std::sync::Arc;
use tokio::time::Duration;

pub struct PlayAction {
    priority: i32,
    clip_queue: Arc<ClipQueue>,
    timeout: Option<Duration>,
    samples: Arc<SampleBuffer>,
}

impl PlayAction {
    pub fn new(
        clip_queue: Arc<ClipQueue>,
        priority: i32,
        timeout: Option<Duration>,
        samples: Arc<SampleBuffer>,
    ) -> PlayAction {
        PlayAction {
            priority,
            clip_queue,
            timeout,
            samples,
        }
    }
}

impl Action for PlayAction {
    fn run(&self) -> ActionFuture {
        let clip_queue = self.clip_queue.clone();
        let samples = self.samples.clone();
        let priority = self.priority;
        let timeout = self.timeout;
        Box::pin(async move {
            clip_queue.play(samples, priority, timeout).await?;
            Ok(())
        })
    }
}
