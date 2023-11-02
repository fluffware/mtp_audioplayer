#[derive(Debug)]
pub enum SampleBuffer {
    I16(Vec<i16>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

impl SampleBuffer {
    pub fn len(&self) -> usize {
        match self {
            SampleBuffer::I16(buf) => buf.len(),
            SampleBuffer::U16(buf) => buf.len(),
            SampleBuffer::F32(buf) => buf.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            SampleBuffer::I16(buf) => buf.is_empty(),
            SampleBuffer::U16(buf) => buf.is_empty(),
            SampleBuffer::F32(buf) => buf.is_empty(),
        }
    }
}

pub trait AsSampleSlice<S> {
    fn as_sample_slice(&self) -> &[S];
}

impl AsSampleSlice<i16> for SampleBuffer {
    fn as_sample_slice(&self) -> &[i16] {
        if let SampleBuffer::I16(buf) = self {
            buf.as_slice()
        } else {
            panic!("SampleBuffer must be I16 for conversion to i16");
        }
    }
}

impl AsSampleSlice<u16> for SampleBuffer {
    fn as_sample_slice(&self) -> &[u16] {
        if let SampleBuffer::U16(buf) = self {
            buf.as_slice()
        } else {
            panic!("SampleBuffer must be U16 for conversion to u16");
        }
    }
}

impl AsSampleSlice<f32> for SampleBuffer {
    fn as_sample_slice(&self) -> &[f32] {
        if let SampleBuffer::F32(buf) = self {
            buf.as_slice()
        } else {
            panic!("SampleBuffer must be F32 for conversion to f32");
        }
    }
}

pub trait Sample {
    const SAMPLE_OFFSET: Self;
    const SAMPLE_MIN: Self;
    const SAMPLE_MAX: Self;
    const SAMPLE_ABS_MAX: Self;
}

impl Sample for i16 {
    const SAMPLE_OFFSET: i16 = 0;
    const SAMPLE_MIN: i16 = -32768;
    const SAMPLE_MAX: i16 = 32767;
    const SAMPLE_ABS_MAX: i16 = 32767;
}

impl Sample for u16 {
    const SAMPLE_OFFSET: u16 = 32768;
    const SAMPLE_MIN: u16 = 0;
    const SAMPLE_MAX: u16 = 65535;
    const SAMPLE_ABS_MAX: u16 = 32767;
}

impl Sample for f32 {
    const SAMPLE_OFFSET: f32 = 0.0;
    const SAMPLE_MIN: f32 = -1.0;
    const SAMPLE_MAX: f32 = 1.0;
    const SAMPLE_ABS_MAX: f32 = 1.0;
}
