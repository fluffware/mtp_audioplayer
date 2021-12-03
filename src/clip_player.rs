use alsa::pcm::PCM;
use alsa::pcm::Format;
use alsa::pcm::Access;
use alsa::pcm::HwParams;
use alsa::Direction;
use alsa::ValueOr;
use alsa::nix::errno::Errno;
use std::ffi::CString;
use std::sync::{Arc};
use std::sync::{Mutex, MutexGuard, Condvar};
use std::sync::atomic::{AtomicU32, Ordering};
use std::future::{self,Future};
use std::task::{Context, Waker, Poll};
use std::thread;
use std::pin::Pin;
use std::time::Duration;
use std::convert::TryFrom;
use log::{debug,error};
use std::ops::Deref;
use std::ops::DerefMut;
use std::mem;


#[derive(Debug, Clone)]
pub struct ClipPlayer
{
    control: Arc<PlaybackControl>
}

#[derive(Debug)]
pub enum Error
{
    Alsa(alsa::Error),
    Shutdown
}

impl std::error::Error for Error {}

impl From<alsa::Error> for Error
{
    fn from(err: alsa::Error) -> Error
    {
        Error::Alsa(err)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
           -> std::result::Result<(), std::fmt::Error>
    {
        match self {
            Error::Alsa(e) => e.fmt(f),
            Error::Shutdown => {
                write!(f,"Playback thread shutdown")
            }
        }
    }
}
        
//const samples: [i16;10000] = [0i16;10000];

#[derive(Debug)]
enum PlaybackState
{
    Setup, // Initializing playback thread
    Ready, // Ready to play samples. Set by thread
    // Play samples. Set by client
    Playing{seqno: u32, samples:Arc<Vec<i16>>},
    Cancel, // Cancel current playback. Set by client
    Error(Error), // Set by thread. Set to Ready to clear
    Shutdown, // Tell the thread to exit.
    Done // The thread has exited
}

impl std::fmt::Display for PlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>)
           -> std::result::Result<(), std::fmt::Error>
    {
        match self {
            PlaybackState::Setup => write!(f,"Setup"),
            PlaybackState::Ready => write!(f,"Ready"),
            PlaybackState::Playing{seqno,samples} => 
                write!(f,"Playing(Seq: {}, Len: {}", seqno, samples.len()),
            PlaybackState::Cancel => write!(f,"Cancel"),
            PlaybackState::Error(e) => write!(f,"Error({})",e),
            PlaybackState::Shutdown => write!(f,"Shutdown"),
            PlaybackState::Done => write!(f,"Done"),
        }
    }
}
        

#[derive(Debug)]
struct PlaybackControl
{
    state: Mutex<PlaybackState>,
    cond: Condvar,
    waker: Mutex<Option<Waker>>
}

impl PlaybackControl
{
    fn change_state(&self,
                    guard: &mut MutexGuard<PlaybackState>, 
                    state: PlaybackState)
                    -> PlaybackState
    {
        let mut state = state;
        debug!("State changed: {}", state);
        mem::swap(guard.deref_mut(), &mut state);

        self.cond.notify_all();
        if let Ok(mut waker) = self.waker.lock() {
            if let Some(waker) = waker.take() {
                waker.wake()
            }
        }
        state
    }

    fn get_state_guard(&self) -> MutexGuard<PlaybackState>
    { 
        match self.state.lock() {
            Ok(g) => g,
            Err(_) => {
                panic!("Playback state thread paniced");
            }
        }
    }
    
}
    
fn play_sample(pcm: &PCM, ctrl: &Arc<PlaybackControl>) -> Result<(), Error>
{
    let wait_delay;
    let channels;
    let frame_rate;
    {
        let hw_params = pcm.hw_params_current()?;
        frame_rate = u64::from(hw_params.get_rate()?);
        wait_delay = 1_000_000_000u64 * u64::try_from(hw_params.get_buffer_size()?).unwrap() / (frame_rate * 2);
        channels = hw_params.get_channels()?;
        
    }
    let mut pos: usize = 0;
    debug!("PCM state: {:?}", pcm.state());
    pcm.drop()?;
    pcm.prepare()?;
    loop {
        match {
            let guard = ctrl.get_state_guard();
            if let PlaybackState::Playing{samples, ..} = guard.deref() {
                
                let s = &samples[pos..];
                if s.is_empty() {break}
                debug!("Writing {}", s.len());
                pcm.io_i16()?.writei(s)
            } else {
                // Playback was canceled
                pcm.drop()?;
                return Ok(())
            }
        } {
            // Check result of write to PCM
            Err(e) => {
                match e.errno() {
                    Errno::EAGAIN => {
                        debug!("Wait");
                        let state = ctrl.get_state_guard();
                        let _state = ctrl.cond.wait_timeout_while(
                            state,
                            Duration::from_nanos(wait_delay),
                            |s| {
                                matches!(s, PlaybackState::Playing{..})
                            }).expect("Failed to wait for pcm buffer");
                    },
                    _ => {
                        pcm.try_recover(e, true)?;
                    },
                }
            },
            Ok(w) => { 
                debug!("Wrote: {}",w);
                pos += w * usize::try_from(channels).unwrap();
            }
        }
        
    }
    
    let state = ctrl.get_state_guard();
    debug!("Wait for clip to finish");
    let delay = u64::try_from(pcm.delay()?).unwrap();
    let left = 1_000_000_000u64 * delay / frame_rate;
    let (_,res) = ctrl.cond.wait_timeout_while(
        state,
        Duration::from_nanos(left),
        |s| {
            matches!(s, PlaybackState::Playing{..})
        }).expect("Failed to wait for clip completion");
        
    if res.timed_out() {
        debug!("Playback finished");
        pcm.drain()?;
    } else {
        debug!("Playback canceled");
        pcm.drop()?;
    }
    Ok(())
}


fn playback_thread(pcm: PCM, ctrl: Arc<PlaybackControl>)
{
    // Ready to play clips
    
    {
        let mut guard = ctrl.get_state_guard();
        ctrl.change_state(&mut guard, PlaybackState::Ready);
    }

    'main:
    loop {
        {
            let mut guard = ctrl.get_state_guard();
            loop {
                match &*guard {
                    PlaybackState::Shutdown => break 'main,
                    PlaybackState::Playing{..} => break,
                    _ => {}
                }
                guard = ctrl.cond.wait(guard)
                    .expect("Failed to wait for state change");
            }
        }
        match play_sample(&pcm, &ctrl) {
            Err(e) => {
                error!("Playback failed: {}",e);
                if let  Ok(mut state) = ctrl.state.lock() {
                    ctrl.change_state(&mut state, PlaybackState::Error(e));
                }
            },
            Ok(_) => {
                if let Ok(mut state) = ctrl.state.lock() {
                    ctrl.change_state(&mut state, PlaybackState::Ready);
                }
                debug!("Clip done");
            }
        }
    }
    {
        let mut guard = ctrl.get_state_guard();
        ctrl.change_state(&mut guard, PlaybackState::Done);
    }
}

struct PlaybackFuture {
    seqno: u32,
    control: Arc<PlaybackControl>
}
impl PlaybackFuture
{
    fn new(seqno: u32, control: Arc<PlaybackControl>) -> PlaybackFuture
    {
        PlaybackFuture{seqno, control}
    }
}

impl Future for PlaybackFuture
{
    type Output = Result<(), Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>
    {
        let ctrl = &self.control;
        let mut guard =  ctrl.get_state_guard();
            
        match &*guard {
            PlaybackState::Error(_) => {
                let state = PlaybackState::Ready;
                let state = ctrl.change_state(&mut guard, state);
                if let PlaybackState::Error(err) = state {
                    Poll::Ready(Err(err))
                } else {
                    panic!("Wrong state");
                }
            },
            PlaybackState::Playing{seqno, ..} 
            if self.seqno == *seqno => {
                let mut waker = ctrl.waker.lock()
                    .expect("Failed to lock waker");
                *waker = Some(cx.waker().clone());
                debug!("Playback future waiting for completion");
                Poll::Pending
            },
            _ => {
                Poll::Ready(Ok(()))
            }
        }
    }
        
}

impl Drop for PlaybackFuture
{
    fn drop(&mut self)
    {
        let ctrl = &self.control;
        let mut guard =  ctrl.get_state_guard();
        if let PlaybackState::Playing{seqno, ..} =  &*guard {
            if self.seqno == *seqno {
                ctrl.change_state(&mut guard, PlaybackState::Cancel);
            }
        }
    }
}

static NEXT_SEQ_NO: AtomicU32 = AtomicU32::new(1);

impl ClipPlayer
{
    pub fn new(pcm_name: &str, rate: u32, channels: u8) 
               -> Result<ClipPlayer, Error>
    {
        let pcm_name = CString::new(pcm_name).unwrap();
        let pcm = PCM::open(pcm_name.as_c_str(), Direction::Playback,false)?;
        {
            let hw_params = HwParams::any(&pcm)?;
            hw_params.set_rate(rate, ValueOr::Nearest)?;
            hw_params.set_channels(u32::from(channels))?;
            hw_params.set_format(Format::s16())?;
            hw_params.set_access(Access::RWInterleaved)?;
            hw_params.set_buffer_size_near(i64::from(rate))?;
            pcm.hw_params(&hw_params)?;
        }
        let control = Arc::new(
            PlaybackControl {
                state: Mutex::new(PlaybackState::Setup),
                cond: Condvar::new(),
                waker: Mutex::new(None)
            });
        let thread_ctrl = control.clone();
        thread::spawn(move || {
            debug!("Started playback thread");
            playback_thread(pcm, thread_ctrl);
        });
        debug!("PCM setup done");
        Ok(ClipPlayer{
            control
        })
    }

    
    pub fn start_clip(&self, clip: Arc<Vec<i16>>)
                            -> Pin<Box<dyn Future<Output = Result<(), Error>>>>
    {
        let seqno = NEXT_SEQ_NO.fetch_add(1, Ordering::Relaxed);
        {
        let mut guard = self.control.get_state_guard();

        loop {
            match &*guard {
                PlaybackState::Setup | PlaybackState::Cancel => {
                    guard = self.control.cond.wait(guard)
                        .expect("Failed to wait for playback thread");
                },
                PlaybackState::Playing{..} => {
                    self.control.change_state(&mut guard,
                                              PlaybackState::Cancel);
                },
                PlaybackState::Ready => break,
                PlaybackState::Error(_) => {
                    let state = self.control.change_state(&mut guard,
                                                          PlaybackState::Ready);
                    if let PlaybackState::Error(err) = state {
                        return Box::pin(future::ready(Err(err)))
                    } else {
                        panic!("Wrong state");
                    }
                },
                PlaybackState::Shutdown | PlaybackState::Done => {
                    return Box::pin(future::ready(Err(Error::Shutdown)))
                }
            }
        }

        self.control.change_state(&mut guard,
                                  PlaybackState::Playing{seqno, samples: clip});
        
        }
        
        Box::pin(PlaybackFuture::new(seqno, self.control.clone()))
    }

    pub fn shutdown(&self)
    {
        let mut guard = self.control.get_state_guard();

        loop {
            match &*guard {
        
                PlaybackState::Done => {
                    return
                },
                PlaybackState::Shutdown => {
                    guard = self.control.cond.wait(guard)
                        .expect("Failed to wait fo shutdown");
                },
                _ => {
                    self.control.change_state(&mut guard,
                                              PlaybackState::Shutdown);
                },
            }
        } 
    }
}
