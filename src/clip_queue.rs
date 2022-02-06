use crate::clip_player::ClipPlayer;
use crate::priority_scheduler::Scheduler;
use std::sync::Arc;
use std::error::Error;
use tokio::time::Duration;
use crate::sample_buffer::SampleBuffer;

pub struct ClipQueue
{
    clip_player: ClipPlayer,
    scheduler: Arc<Scheduler>
}

impl ClipQueue
{
    pub fn new(clip_player: ClipPlayer) -> ClipQueue
    {
	ClipQueue{clip_player,
		  scheduler: Scheduler::new()
	}
    }
    
    pub async fn play(&self, samples: Arc<SampleBuffer>, priority: i32, 
		      timeout: Option<Duration>) ->
		      Result<(), Box<dyn Error + Send + Sync>>
    {
	let token;
	if let Some(timeout) = timeout {
	    token = match self.scheduler.get_token_timeout(priority, timeout).await {
		Some(t) => t,
		None => return Ok(())
	    }
	} else {
	    token = self.scheduler.get_token(priority).await;
	}
	self.clip_player.start_clip(samples).await?;
	drop(token);
	Ok(())
    }
}
