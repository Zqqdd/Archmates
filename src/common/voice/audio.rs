//! Microphone recording and speaker playback (cpal), for voice messages.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};

use super::{MAX_SECS, SAMPLE_RATE};

/// Recording from the default microphone. Stops by itself after 5 minutes.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    started: Instant,
    thread: Option<JoinHandle<Result<Vec<u8>>>>,
    level: Arc<Mutex<f32>>,
}

impl Recorder {
    pub fn start() -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(Mutex::new(0.0));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let (stop2, level2) = (stop.clone(), level.clone());
        // cpal streams can't move between threads: the whole recording lives in this one.
        let thread = std::thread::spawn(move || record(stop2, level2, ready_tx));
        ready_rx.recv().map_err(|_| anyhow!("the microphone could not start"))??;
        Ok(Self { stop, started: Instant::now(), thread: Some(thread), level })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Microphone level, 0..1 (for a small meter).
    pub fn level(&self) -> f32 {
        *self.level.lock().unwrap()
    }

    /// True once the 5-minute limit is reached (the recording has stopped).
    pub fn is_full(&self) -> bool {
        self.started.elapsed() >= Duration::from_secs(MAX_SECS as u64)
    }

    /// Stops and returns the recording as WAV bytes.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().map_err(|_| anyhow!("recording crashed"))?
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn record(
    stop: Arc<AtomicBool>,
    level: Arc<Mutex<f32>>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) -> Result<Vec<u8>> {
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let started = (|| -> Result<(cpal::Stream, u32, u16)> {
        let device = cpal::default_host().default_input_device().ok_or_else(|| anyhow!("no microphone found"))?;
        let config = device.default_input_config().context("the microphone can't be used")?;
        let (rate, channels) = (config.sample_rate().0, config.channels());
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => input::<f32>(&device, &config.into(), samples.clone(), level.clone()),
            cpal::SampleFormat::I16 => input::<i16>(&device, &config.into(), samples.clone(), level.clone()),
            cpal::SampleFormat::U16 => input::<u16>(&device, &config.into(), samples.clone(), level.clone()),
            cpal::SampleFormat::I32 => input::<i32>(&device, &config.into(), samples.clone(), level.clone()),
            other => Err(anyhow!("unsupported microphone format {other}")),
        }?;
        stream.play()?;
        Ok((stream, rate, channels))
    })();
    let (stream, rate, channels) = match started {
        Ok(s) => {
            let _ = ready.send(Ok(()));
            s
        }
        Err(e) => {
            let _ = ready.send(Err(anyhow!("{e:#}")));
            return Err(e);
        }
    };
    let begin = Instant::now();
    while !stop.load(Ordering::SeqCst) && begin.elapsed() < Duration::from_secs(MAX_SECS as u64) {
        std::thread::sleep(Duration::from_millis(30));
    }
    drop(stream);
    let raw = std::mem::take(&mut *samples.lock().unwrap());
    let mono: Vec<f32> = raw.chunks(channels as usize).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let mut pcm = resample(&mono, rate, SAMPLE_RATE);
    pcm.truncate((MAX_SECS * SAMPLE_RATE) as usize);
    encode_wav(&pcm)
}

fn input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    level: Arc<Mutex<f32>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let peak = data.iter().map(|s| f32::from_sample(*s).abs()).fold(0.0, f32::max);
            *level.lock().unwrap() = peak.min(1.0);
            samples.lock().unwrap().extend(data.iter().map(|s| f32::from_sample(*s)));
        },
        |err| eprintln!("microphone: {err}"),
        None,
    )?;
    Ok(stream)
}

/// Linear resampling: good enough for speech.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = (input.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos as usize;
            let frac = (pos - j as f64) as f32;
            let a = input[j.min(input.len() - 1)];
            let b = input[(j + 1).min(input.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

fn encode_wav(pcm: &[f32]) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut out, spec)?;
        for s in pcm {
            writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        writer.finalize()?;
    }
    Ok(out.into_inner())
}

/// Plays a voice message on the default speakers.
pub struct Player {
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    position: Arc<Mutex<f32>>,
    pub secs: f32,
}

impl Player {
    pub fn play(path: &std::path::Path) -> Result<Self> {
        let mut reader = hound::WavReader::open(path).context("this voice message has expired")?;
        let spec = reader.spec();
        let pcm: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                reader.samples::<i16>().filter_map(|s| s.ok()).map(|s| s as f32 / i16::MAX as f32).collect()
            }
            hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        };
        let mono: Vec<f32> =
            pcm.chunks(spec.channels.max(1) as usize).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
        let secs = mono.len() as f32 / spec.sample_rate as f32;
        let (stop, done, position) =
            (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(0.0)));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let (s2, d2, p2, rate) = (stop.clone(), done.clone(), position.clone(), spec.sample_rate);
        std::thread::spawn(move || {
            let result = play(&mono, rate, &s2, &p2, &ready_tx);
            if let Err(e) = result {
                let _ = ready_tx.send(Err(e));
            }
            d2.store(true, Ordering::SeqCst);
        });
        ready_rx.recv().map_err(|_| anyhow!("the speakers could not start"))??;
        Ok(Self { stop, done, position, secs })
    }

    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// Seconds played so far.
    pub fn position(&self) -> f32 {
        *self.position.lock().unwrap()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Blocks until the end (for the command line).
    pub fn wait(&self) {
        while !self.is_done() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

fn play(
    mono: &[f32],
    rate: u32,
    stop: &AtomicBool,
    position: &Mutex<f32>,
    ready: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| anyhow!("no speakers found"))?;
    let config = device.default_output_config().context("the speakers can't be used")?;
    let (out_rate, channels) = (config.sample_rate().0, config.channels() as usize);
    let samples = Arc::new(resample(mono, rate, out_rate));
    let cursor = Arc::new(Mutex::new(0usize));
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => output::<f32>(&device, &config.into(), samples.clone(), cursor.clone(), channels),
        cpal::SampleFormat::I16 => output::<i16>(&device, &config.into(), samples.clone(), cursor.clone(), channels),
        cpal::SampleFormat::U16 => output::<u16>(&device, &config.into(), samples.clone(), cursor.clone(), channels),
        cpal::SampleFormat::I32 => output::<i32>(&device, &config.into(), samples.clone(), cursor.clone(), channels),
        other => Err(anyhow!("unsupported speaker format {other}")),
    }?;
    stream.play()?;
    let _ = ready.send(Ok(()));
    loop {
        let at = *cursor.lock().unwrap();
        *position.lock().unwrap() = at as f32 / out_rate as f32;
        if stop.load(Ordering::SeqCst) || at >= samples.len() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    // Let the last buffer drain.
    std::thread::sleep(Duration::from_millis(150));
    Ok(())
}

fn output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Vec<f32>>,
    cursor: Arc<Mutex<usize>>,
    channels: usize,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut at = cursor.lock().unwrap();
            for frame in data.chunks_mut(channels) {
                let s = samples.get(*at).copied().unwrap_or(0.0);
                *at += 1;
                for out in frame.iter_mut() {
                    *out = T::from_sample(s);
                }
            }
        },
        |err| eprintln!("speakers: {err}"),
        None,
    )?;
    Ok(stream)
}
