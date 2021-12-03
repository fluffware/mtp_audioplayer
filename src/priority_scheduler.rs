use tokio::sync::Notify;
use std::sync::Mutex;
use std::sync::Arc;
use std::vec::Vec;
use tokio::time::{Instant,Duration};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone)]
struct TokenState
{
    id: u32,
    priority: i32,
    notify: Arc<Notify>,
}
    
    
pub struct Token
{
    id: u32,
    scheduler: Arc<Scheduler>
}

impl Token
{
    pub fn release(self: &Token) 
    {

	self.scheduler.release(self.id);
    }

    pub fn is_released(&self) -> bool
    {
	self.scheduler.is_released(self.id)
    }
    
    pub fn is_active(&self) -> bool
    {
	self.scheduler.is_active(self.id)
    }

    pub fn is_waiting(&self) -> bool
    {
	self.scheduler.is_waiting(self.id)
    }

    
    pub async fn wait_release(&self)
    {
	while !self.is_released() {
	    if let Some(notify) = self.scheduler.get_notify(self.id) {
		notify.notified().await;
	    }
	}
    }

}


impl Drop for Token
{
    fn drop(&mut self)
    {
	self.release();
    }
}

pub struct Scheduler
{
    queue: Mutex<Vec<TokenState>>
}

fn find_id(states: &[TokenState], id: u32) -> Option<usize>
{
    for (i, state) in states.iter().enumerate() {
	if state.id == id {
	    return Some(i);
	}
    }
    None
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

impl Scheduler
{
    pub fn new() -> Arc<Scheduler>
    {
	Arc::new(Scheduler{queue: Mutex::new(Vec::new())})
    }

    fn release(self :&Arc<Scheduler>, id: u32)
    {
	let mut queue = self.queue.lock().unwrap();
	if let Some(index ) = find_id(&queue, id) {
	    let state = queue.remove(index);
	    // Notify the token that it was removed
	    state.notify.notify_one();
	    // If we removed the active token, notify the next token
	    // that it's active
	    if index == 0 && !queue.is_empty() {
		queue[0].notify.notify_one();
	    }
	}
	    
    }

    fn is_active(self: &Arc<Scheduler>, id: u32) -> bool
    {
	let queue = &mut self.queue.lock().unwrap();
	queue[0].id == id
    }

    fn is_waiting(self: &Arc<Scheduler>, id: u32) -> bool
    {
	let queue = &mut self.queue.lock().unwrap();
	matches!(find_id(queue, id), Some(id) if id > 0)
    }

    fn is_released(self: &Arc<Scheduler>, id: u32) -> bool
    {
	let queue = &mut self.queue.lock().unwrap();
	find_id(queue, id).is_none()
    }

    fn get_notify(self: &Arc<Scheduler>, id: u32) -> Option<Arc<Notify>>
    {
	let queue = &mut self.queue.lock().unwrap();
	find_id(queue, id).map(|index| queue[index].notify.clone())
    }

    fn get_token_with_notify(self: &Arc<Scheduler>, priority: i32) 
			 -> (Token, Arc<Notify>)
    {
	let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
	let notify = Arc::new(Notify::new());
	let state = TokenState{id, priority, notify: notify.clone()};
	{
	    let queue = &mut self.queue.lock().unwrap();
	    queue.push(state);
	    queue.sort_by(|s1, s2| s2.priority.cmp(&s1.priority));
	    // If this token is active then the second token was pushed away
	    // and should be released
	    if queue[0].id == id && queue.len() >= 2{
		let state = queue.remove(1);
		state.notify.notify_one();
	    }
	}
	(Token{id, scheduler: self.clone()}, notify)
    }
    
    pub async fn get_token(self: &Arc<Scheduler>, priority: i32) -> Token
    {
	
	let (token, notify) = self.get_token_with_notify(priority);
	while token.is_waiting() {
	    notify.notified().await;
	}
	token
    }

    pub async fn get_token_timeout(self: &Arc<Scheduler>, priority: i32, t: Duration) -> Option<Token>
    {
	let (token, notify) = self.get_token_with_notify(priority);
	println!("get_token_timeout: Waiting for {:?}", t);
	let end = Instant::now() + t;
	while token.is_waiting() {
	    match tokio::time::timeout_at(end, notify.notified()).await {
		Ok(_) => {},
		Err(_) => return None
	    }
	}
	Some(token)
    }

}

#[tokio::test]
async fn test_equal_prority()
{
    let sched = Scheduler::new();
    let sched1 = sched.clone();
    tokio::spawn(async move {
	let token1 = sched1.get_token(3).await;
	match tokio::time::timeout(Duration::from_millis(500),
				   token1.wait_release()).await {
	    Ok(_) => println!("Forced release token1"),
	    Err(_) => println!("Release token1")
	}
	token1.release();
    });
    let sched2 = sched.clone();
    tokio::spawn(async move {
	let token2 = sched2.get_token(3).await;
	match tokio::time::timeout(Duration::from_millis(500),
				   token2.wait_release()).await {
	    Ok(_) => println!("Forced release token2"),
	    Err(_) => println!("Release token2")
	}
    });
    let sched3 = sched.clone();
    tokio::spawn(async move {
	let token = sched3.get_token(3).await;
	
	match tokio::time::timeout(Duration::from_millis(500),
				   token.wait_release()).await {
	    Ok(_) => println!("Forced release token3"),
	    Err(_) => println!("Release token3")
	}
	token.release();
    });
    tokio::time::sleep(Duration::from_millis(2000)).await;
}

#[tokio::test]
async fn test_higher_prority()
{
    let sched = Scheduler::new();
    let sched1 = sched.clone();
    tokio::spawn(async move {
	let token1 = sched1.get_token(3).await;
	match tokio::time::timeout(Duration::from_millis(3000),
				   token1.wait_release()).await {
	    Ok(_) => println!("Forced release token1"),
	    Err(_) => println!("Release token1")
	}
    });
    let sched2 = sched.clone();
    tokio::spawn(async move {
	tokio::time::sleep(Duration::from_millis(500)).await;
	let token2 = sched2.get_token(4).await;
	tokio::time::sleep(Duration::from_millis(500)).await;
	match tokio::time::timeout(Duration::from_millis(1000),
				   token2.wait_release()).await {
	    Ok(_) => println!("Forced release token2"),
	    Err(_) => println!("Release token2")
	}
    });
    let sched3 = sched.clone();
    tokio::spawn(async move {
	let token = sched3.get_token(4).await;
	tokio::time::sleep(Duration::from_millis(300)).await;
	token.release();
	println!("Release token3");
    });
    tokio::time::sleep(Duration::from_millis(3000)).await;
}

