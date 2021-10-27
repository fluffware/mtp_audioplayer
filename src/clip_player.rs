use alsa::pcm::PCM;
use alsa::pcm::Format;
use alsa::pcm::Access;
use alsa::pcm::HwParams;
use alsa::Direction;
use alsa::ValueOr;
use alsa::nix::errno::Errno;

use std::ffi::CString;
use std::sync::{Arc};
use std::sync::mpsc::{self, Sender, Receiver,RecvTimeoutError};

use std::thread;
use std::time::Duration;
use std::convert::TryFrom;
use log::{debug,error};

#[derive(Debug)]
enum ClipCmd
{
    Play(Arc<Vec<i16>>),
    Stop
}

#[derive(Debug, Clone)]
pub struct ClipPlayer
{
    cmd_channel: Sender<ClipCmd>
}

//const samples: [i16;10000] = [0i16;10000];

fn play_sample(pcm: &PCM, cmd: &Receiver<ClipCmd>, samples: &Arc<Vec<i16>>)
                -> alsa::Result<Option<ClipCmd>>
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
    let mut received = None;
    let mut pos: usize = 0;
    debug!("PCM state: {:?}", pcm.state());
    pcm.drop()?;
    pcm.prepare()?;
    loop {
        match {
            let s = &samples[pos..];
            if s.is_empty() {break}
            debug!("Writing {}", s.len());
            pcm.io_i16()?.writei(s) 
        }
        {
            Err(e) => {
                match e.errno() {
                    Some(Errno::EAGAIN) => {
                        debug!("Wait");
                        match cmd.recv_timeout(
                            Duration::from_nanos(wait_delay)) {
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => break,
                            Ok(cmd) => {
                                //debug!("Got command {:?}", cmd);
                                received = Some(cmd);
                                break;
                            }
                        };
                    },
                    Some(_) => {
                        pcm.try_recover(e, true)?;
                    },
                    _ => return Err(e.into())
                }
            },
            Ok(w) => { 
                debug!("Wrote: {}",w);
                pos += w * usize::try_from(channels).unwrap();
            }
        }
    }
    if !received.is_some() {
        // Wait for clip to finish
        debug!("Wait for clip to finish");

        let delay = u64::try_from(pcm.delay()?).unwrap();
        let left = 1_000_000_000u64 * delay / frame_rate;
        if let Ok(cmd) = cmd.recv_timeout(Duration::from_nanos(left)) {
            //debug!("Got command {:?}", cmd);
            received = Some(cmd);
        }
    } else {
        pcm.drop()?;
    }
    Ok(received)
}

fn playback_thread(pcm: PCM, cmd_channel: Receiver<ClipCmd>) -> alsa::Result<()>
{
   
    let mut recv = cmd_channel.recv().ok();
    while let Some(ref cmd) = recv {
        match cmd {
            ClipCmd::Play(samples) => {
                match play_sample(&pcm, &cmd_channel, &samples)? {
                    None => {
                        recv = None;
                    },
                    Some(r) => {
                        recv = Some(r);
                    }
                }
                debug!("Clip done");
            },
            ClipCmd::Stop => {}
        }
        if recv.is_none() {
            recv = cmd_channel.recv().ok();
        }
    }
        
    Ok(())
}

impl ClipPlayer
{
    pub fn new(pcm_name: &str, rate: u32, channels: u8) 
               -> alsa::Result<ClipPlayer>
    {
        let pcm_name = CString::new(pcm_name).unwrap();
        let pcm = PCM::open(pcm_name.as_c_str(), Direction::Playback,true)?;
        {
            let hw_params = HwParams::any(&pcm)?;
            hw_params.set_rate(rate, ValueOr::Nearest)?;
            hw_params.set_channels(u32::from(channels))?;
            hw_params.set_format(Format::s16())?;
            hw_params.set_access(Access::RWInterleaved)?;
            hw_params.set_buffer_size_near(i64::from(rate))?;
            pcm.hw_params(&hw_params)?;
        }
        let (send, recv) = mpsc::channel();
        thread::spawn(move || {
            debug!("Started playback thread");
            if let Err(e) = playback_thread(pcm, recv) {
                error!("Playback failed: {}",e);
            }
        });
        debug!("PCM setup done");
        Ok(ClipPlayer{
            cmd_channel: send
        })
    }

    pub fn start_clip(&self, clip: Arc<Vec<i16>>)
                      -> alsa::Result<()>
    {
        self.cmd_channel.send(ClipCmd::Play(clip)).unwrap();
        Ok(())
    }
}
