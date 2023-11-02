use crate::sample_buffer::{self, AsSampleSlice, SampleBuffer};
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use cpal::BuildStreamError;
use cpal::Device;
use cpal::SampleFormat;
use cpal::SampleRate;
use cpal::Stream;
use cpal::StreamConfig;
use cpal::SupportedStreamConfigRange;
use log::{debug, error, info};
use std::future::{self, Future};
use std::mem;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::thread;

#[derive(Debug, Clone)]
pub struct ClipPlayer {
    control: Arc<PlaybackControl>,
}

#[derive(Debug)]
pub enum Error {
    Devices(cpal::DevicesError),
    Name(cpal::DeviceNameError),
    BuildStream(cpal::BuildStreamError),
    PlayStream(cpal::PlayStreamError),
    SupportedConfig(cpal::SupportedStreamConfigsError),
    NoMatchinConfig(String),
    ClipPlayer(String),
    Shutdown,
}

impl std::error::Error for Error {}

impl From<cpal::DevicesError> for Error {
    fn from(err: cpal::DevicesError) -> Error {
        Error::Devices(err)
    }
}

impl From<cpal::DeviceNameError> for Error {
    fn from(err: cpal::DeviceNameError) -> Error {
        Error::Name(err)
    }
}

impl From<cpal::BuildStreamError> for Error {
    fn from(err: cpal::BuildStreamError) -> Error {
        Error::BuildStream(err)
    }
}

impl From<cpal::SupportedStreamConfigsError> for Error {
    fn from(err: cpal::SupportedStreamConfigsError) -> Error {
        Error::SupportedConfig(err)
    }
}

impl From<cpal::PlayStreamError> for Error {
    fn from(err: cpal::PlayStreamError) -> Error {
        Error::PlayStream(err)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Error {
        Error::ClipPlayer(s)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Error::Devices(e) => e.fmt(f),
            Error::Name(e) => e.fmt(f),
            Error::BuildStream(e) => e.fmt(f),
            Error::PlayStream(e) => e.fmt(f),
            Error::ClipPlayer(e) => e.fmt(f),
            Error::SupportedConfig(e) => e.fmt(f),
            Error::NoMatchinConfig(e) => e.fmt(f),
            Error::Shutdown => {
                write!(f, "Playback thread shutdown")
            }
        }
    }
}

//const samples: [i16;10000] = [0i16;10000];

#[derive(Debug)]
enum PlaybackState {
    Setup, // Initializing playback thread
    Ready, // Ready to play samples. Set by thread
    // Play samples. Set by client
    Playing {
        seqno: u32,
        samples: Arc<SampleBuffer>,
    },
    Cancel, // Cancel current playback. Set by client
    #[allow(dead_code)]
    Error(Error), // Set by thread. Set to Ready to clear
    Shutdown, // Tell the thread to exit.
    Done,   // The thread has exited
}

impl std::fmt::Display for PlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            PlaybackState::Setup => write!(f, "Setup"),
            PlaybackState::Ready => write!(f, "Ready"),
            PlaybackState::Playing { seqno, samples } => {
                write!(f, "Playing(Seq: {}, Len: {}", seqno, samples.len())
            }
            PlaybackState::Cancel => write!(f, "Cancel"),
            PlaybackState::Error(e) => write!(f, "Error({})", e),
            PlaybackState::Shutdown => write!(f, "Shutdown"),
            PlaybackState::Done => write!(f, "Done"),
        }
    }
}

struct PlaybackControl {
    state: Mutex<PlaybackState>,
    cond: Condvar,
    waker: Mutex<Option<Waker>>,
}

impl std::fmt::Debug for PlaybackControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "PlaybackControl{{state: {:?}, cond: {:?}, waker: {:?}}}",
            self.state, self.cond, self.waker
        )
    }
}

impl PlaybackControl {
    fn change_state(
        &self,
        guard: &mut MutexGuard<PlaybackState>,
        state: PlaybackState,
    ) -> PlaybackState {
        let mut state = state;
        //debug!("State changed: {}", state);
        mem::swap(guard.deref_mut(), &mut state);

        self.cond.notify_all();
        if let Ok(mut waker) = self.waker.lock() {
            if let Some(waker) = waker.take() {
                waker.wake()
            }
        }
        state
    }

    fn get_state_guard(&self) -> MutexGuard<PlaybackState> {
        match self.state.lock() {
            Ok(g) => g,
            Err(_) => {
                panic!("Playback state thread paniced");
            }
        }
    }
}
fn generate_samples<S>(
    ctrl: &PlaybackControl,
    buffer: &mut [S],
    current_seqno: &mut u32,
    pos: &mut usize,
) where
    S: sample_buffer::Sample + Copy,
    SampleBuffer: AsSampleSlice<S>,
{
    if let Ok(mut state) = ctrl.state.lock() {
        match &mut *state {
            PlaybackState::Playing { seqno, samples } => {
                let samples: &[S] = samples.as_sample_slice();
                if *seqno != *current_seqno {
                    *current_seqno = *seqno;
                    *pos = 0;
                }
                if *pos >= samples.len() {
                    *pos = 0;
                }
                //debug!("{} @ {}", *seqno, pos);
                if samples.len() - *pos >= buffer.len() {
                    let end = *pos + buffer.len();
                    buffer.copy_from_slice(&samples[*pos..end]);
                    *pos = end;
                } else {
                    let end = samples.len();
                    let copy_len = end - *pos;
                    buffer[0..copy_len].copy_from_slice(&samples[*pos..end]);
                    for s in buffer[copy_len..].iter_mut() {
                        *s = S::SAMPLE_OFFSET;
                    }
                    *pos = end;
                }
                if *pos >= samples.len() {
                    *pos = 0;
                    ctrl.change_state(&mut state, PlaybackState::Ready);
                    //debug!("Stream callback: Done");
                }
            }
            PlaybackState::Cancel => {
                *pos = 0;
                ctrl.change_state(&mut state, PlaybackState::Ready);
            }
            _ => {
                //debug!("Stream callback: Silence");
                for s in buffer {
                    *s = S::SAMPLE_OFFSET;
                }
            }
        }
    }
}

fn build_output_stream<S>(
    device: Device,
    stream_config: &StreamConfig,
    sample_format: SampleFormat,
    ctrl_cb: Arc<PlaybackControl>,
) -> Result<Stream, BuildStreamError>
where
    S: cpal::Sample + Copy + sample_buffer::Sample,
    SampleBuffer: AsSampleSlice<S>,
{
    let mut current_seqno = 0;
    let mut pos = 0;
    device.build_output_stream_raw(
        stream_config,
        sample_format,
        move |data, _info| {
            let buffer = data.as_slice_mut::<S>().unwrap();
            generate_samples::<S>(ctrl_cb.as_ref(), buffer, &mut current_seqno, &mut pos);
        },
        |err| {
            error!("Stream error: {}", err);
        },
    )
}
fn playback_thread(
    device: Device,
    stream_config: StreamConfig,
    sample_format: SampleFormat,
    ctrl: Arc<PlaybackControl>,
) {
    let ctrl_cb = ctrl.clone();
    let stream = match match sample_format {
        SampleFormat::I16 => {
            build_output_stream::<i16>(device, &stream_config, sample_format, ctrl_cb)
        }
        SampleFormat::U16 => {
            build_output_stream::<u16>(device, &stream_config, sample_format, ctrl_cb)
        }
        SampleFormat::F32 => {
            build_output_stream::<f32>(device, &stream_config, sample_format, ctrl_cb)
        }
    } {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to initiate audio playback: {}", e);
            return;
        }
    };
    if let Err(e) = stream.play() {
        error!("Failed to start audio playback: {}", e);
        return;
    }

    let mut guard = ctrl.get_state_guard();
    ctrl.change_state(&mut guard, PlaybackState::Ready);
    loop {
        if let PlaybackState::Shutdown = &*guard {
            break;
        }
        guard = ctrl
            .cond
            .wait(guard)
            .expect("Failed to wait for state change");
    }
    ctrl.change_state(&mut guard, PlaybackState::Done);
    debug!("Playback thread exited");
}

struct PlaybackFuture {
    seqno: u32,
    control: Arc<PlaybackControl>,
}
impl PlaybackFuture {
    fn new(seqno: u32, control: Arc<PlaybackControl>) -> PlaybackFuture {
        PlaybackFuture { seqno, control }
    }
}

impl Future for PlaybackFuture {
    type Output = Result<(), Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ctrl = &self.control;
        let mut guard = ctrl.get_state_guard();

        match &*guard {
            PlaybackState::Error(_) => {
                let state = PlaybackState::Ready;
                let state = ctrl.change_state(&mut guard, state);
                if let PlaybackState::Error(err) = state {
                    Poll::Ready(Err(err))
                } else {
                    panic!("Wrong state");
                }
            }
            PlaybackState::Playing { seqno, .. } if self.seqno == *seqno => {
                let mut waker = ctrl.waker.lock().expect("Failed to lock waker");
                *waker = Some(cx.waker().clone());
                //debug!("Playback future waiting for completion");
                Poll::Pending
            }
            _ => Poll::Ready(Ok(())),
        }
    }
}

impl Drop for PlaybackFuture {
    fn drop(&mut self) {
        let ctrl = &self.control;
        let mut guard = ctrl.get_state_guard();
        if let PlaybackState::Playing { seqno, .. } = &*guard {
            if self.seqno == *seqno {
                ctrl.change_state(&mut guard, PlaybackState::Cancel);
            }
        }
    }
}

fn supports_samplerate(conf: &SupportedStreamConfigRange, rate: u32) -> bool {
    conf.min_sample_rate().0 <= rate && conf.max_sample_rate().0 >= rate
}

static NEXT_SEQ_NO: AtomicU32 = AtomicU32::new(1);

impl ClipPlayer {
    pub fn new(
        pcm_name: &str,
        rate: u32,
        channels: u8,
        sample_format: SampleFormat,
    ) -> Result<ClipPlayer, Error> {
        let channels = channels as u16;
        let host = cpal::default_host();
        let device = if pcm_name == "default" {
            host.default_output_device()
                .ok_or_else(|| "No default device".to_string())?
        } else {
            let mut selected = None;
            let devices = host.output_devices()?;
            for device in devices {
                debug!("Checking device {}", device.name()?);
                if device.name()? == pcm_name {
                    selected = Some(device);
                    break;
                }
            }
            selected.ok_or_else(|| format!("Playback device {} not found", pcm_name))?
        };
        info!("Audio playback on device {}", device.name()?);
        let mut best_fit: Option<SupportedStreamConfigRange> = None;
        let supported_configs = device.supported_output_configs()?;
        for conf in supported_configs {
            /*debug!(
                "Config: {}ch, {}-{}samples/s {:?}",
                conf.channels(),
                conf.min_sample_rate().0,
                conf.max_sample_rate().0,
                conf.sample_format()
            );*/
            if let Some(prev) = &best_fit {
                // Check if this conf matches better than the previous best conf
                if (conf.channels() == channels && prev.channels() != channels)
                    || (supports_samplerate(&conf, rate) && !supports_samplerate(prev, rate))
                    || (conf.sample_format() == sample_format
                        && prev.sample_format() != sample_format)
                {
                    best_fit = Some(conf);
                }
            } else {
                best_fit = Some(conf);
            }
        }

        let best_fit = best_fit
            .ok_or_else(|| Error::NoMatchinConfig("No suitable configuration found".to_string()))?;
        if best_fit.channels() != channels {
            return Err(Error::NoMatchinConfig(format!(
                "No configuration with {} channels found",
                channels
            )));
        }
        if !supports_samplerate(&best_fit, rate) {
            return Err(Error::NoMatchinConfig(format!(
                "No configuration that supports {} samples/s found",
                rate
            )));
        }
        if best_fit.sample_format() != sample_format {
            return Err(Error::NoMatchinConfig(
                "No configuration with signed 16-bit format found".to_string(),
            ));
        }
        let stream_config = best_fit.with_sample_rate(SampleRate(rate)).config();
        let control = Arc::new(PlaybackControl {
            state: Mutex::new(PlaybackState::Setup),
            cond: Condvar::new(),
            waker: Mutex::new(None),
        });
        let thread_ctrl = control.clone();
        thread::spawn(move || playback_thread(device, stream_config, sample_format, thread_ctrl));

        Ok(ClipPlayer { control })
    }

    pub fn start_clip(
        &self,
        clip: Arc<SampleBuffer>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>> {
        let seqno = NEXT_SEQ_NO.fetch_add(1, Ordering::Relaxed);
        {
            let mut guard = self.control.get_state_guard();

            loop {
                match &*guard {
                    PlaybackState::Setup | PlaybackState::Cancel => {
                        guard = self
                            .control
                            .cond
                            .wait(guard)
                            .expect("Failed to wait for playback thread");
                    }
                    PlaybackState::Playing { .. } => {
                        self.control.change_state(&mut guard, PlaybackState::Cancel);
                    }
                    PlaybackState::Ready => break,
                    PlaybackState::Error(_) => {
                        let state = self.control.change_state(&mut guard, PlaybackState::Ready);
                        if let PlaybackState::Error(err) = state {
                            return Box::pin(future::ready(Err(err)));
                        } else {
                            panic!("Wrong state");
                        }
                    }
                    PlaybackState::Shutdown | PlaybackState::Done => {
                        return Box::pin(future::ready(Err(Error::Shutdown)))
                    }
                }
            }

            self.control.change_state(
                &mut guard,
                PlaybackState::Playing {
                    seqno,
                    samples: clip,
                },
            );
        }

        Box::pin(PlaybackFuture::new(seqno, self.control.clone()))
    }

    pub fn shutdown(&self) {
        let mut guard = self.control.get_state_guard();

        loop {
            match &*guard {
                PlaybackState::Done => return,
                PlaybackState::Shutdown => {
                    guard = self
                        .control
                        .cond
                        .wait(guard)
                        .expect("Failed to wait fo shutdown");
                }
                _ => {
                    self.control
                        .change_state(&mut guard, PlaybackState::Shutdown);
                }
            }
        }
    }
}
