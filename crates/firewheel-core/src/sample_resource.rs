use std::{num::NonZeroUsize, ops::Range, sync::Arc};

/// A resource of audio samples.
pub trait SampleResource: Send + Sync + 'static {
    /// The number of channels in this resource.
    fn num_channels(&self) -> NonZeroUsize;

    /// The length of this resource in samples (the number of samples
    /// in a single channel).
    fn len_samples(&self) -> u64;

    /// Fill the given buffers with audio data starting from the given
    /// starting frame in the resource.
    ///
    /// * `buffers` - The buffers to fill with data. If the length of `buffers`
    /// is greater than the number of channels in this resource, then ignore
    /// the extra buffers.
    /// * `buffer_range` - The range inside each buffer slice in which to
    /// fill with data. Do not fill any data outside of this range.
    /// * `start_sample` - The sample in the resource at which to start copying
    /// from.
    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    );
}

pub struct InterleavedResourceI16 {
    pub data: Vec<i16>,
    pub channels: NonZeroUsize,
}

impl SampleResource for InterleavedResourceI16 {
    fn num_channels(&self) -> NonZeroUsize {
        self.channels
    }

    fn len_samples(&self) -> u64 {
        (self.data.len() / self.channels.get()) as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_interleaved(
            buffers,
            buffer_range,
            start_sample as usize,
            self.channels,
            &self.data,
            pcm_i16_to_f32,
        );
    }
}

impl SampleResource for Arc<InterleavedResourceI16> {
    fn num_channels(&self) -> NonZeroUsize {
        self.channels
    }

    fn len_samples(&self) -> u64 {
        (self.data.len() / self.channels.get()) as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_interleaved(
            buffers,
            buffer_range,
            start_sample as usize,
            self.channels,
            &self.data,
            pcm_i16_to_f32,
        );
    }
}

pub struct InterleavedResourceU16 {
    pub data: Vec<u16>,
    pub channels: NonZeroUsize,
}

impl SampleResource for InterleavedResourceU16 {
    fn num_channels(&self) -> NonZeroUsize {
        self.channels
    }

    fn len_samples(&self) -> u64 {
        (self.data.len() / self.channels.get()) as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_interleaved(
            buffers,
            buffer_range,
            start_sample as usize,
            self.channels,
            &self.data,
            pcm_u16_to_f32,
        );
    }
}

pub struct InterleavedResourceF32 {
    pub data: Vec<f32>,
    pub channels: NonZeroUsize,
}

impl SampleResource for InterleavedResourceF32 {
    fn num_channels(&self) -> NonZeroUsize {
        self.channels
    }

    fn len_samples(&self) -> u64 {
        (self.data.len() / self.channels.get()) as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_interleaved(
            buffers,
            buffer_range,
            start_sample as usize,
            self.channels,
            &self.data,
            |s| s,
        );
    }
}

impl SampleResource for Vec<Vec<i16>> {
    fn num_channels(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.len()).unwrap()
    }

    fn len_samples(&self) -> u64 {
        self[0].len() as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_deinterleaved(
            buffers,
            buffer_range,
            start_sample as usize,
            self.as_slice(),
            pcm_i16_to_f32,
        );
    }
}

impl SampleResource for Vec<Vec<u16>> {
    fn num_channels(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.len()).unwrap()
    }

    fn len_samples(&self) -> u64 {
        self[0].len() as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_deinterleaved(
            buffers,
            buffer_range,
            start_sample as usize,
            self.as_slice(),
            pcm_u16_to_f32,
        );
    }
}

impl SampleResource for Vec<Vec<f32>> {
    fn num_channels(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.len()).unwrap()
    }

    fn len_samples(&self) -> u64 {
        self[0].len() as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_deinterleaved_f32(buffers, buffer_range, start_sample as usize, self);
    }
}

#[inline]
pub fn pcm_i16_to_f32(s: i16) -> f32 {
    f32::from(s) * (1.0 / std::i16::MAX as f32)
}

#[inline]
pub fn pcm_u16_to_f32(s: u16) -> f32 {
    ((f32::from(s)) * (2.0 / std::u16::MAX as f32)) - 1.0
}

/// A helper method to fill buffers from a resource of interleaved samples.
pub fn fill_buffers_interleaved<T: Clone + Copy>(
    buffers: &mut [&mut [f32]],
    buffer_range: Range<usize>,
    start_sample: usize,
    channels: NonZeroUsize,
    data: &[T],
    convert: impl Fn(T) -> f32,
) {
    let start_sample = start_sample as usize;
    let channels = channels.get();

    let samples = buffer_range.end - buffer_range.start;

    if channels == 1 {
        // Mono, no need to deinterleave.
        for (buf_s, &src_s) in buffers[0][buffer_range.clone()]
            .iter_mut()
            .zip(&data[start_sample..start_sample + samples])
        {
            *buf_s = convert(src_s);
        }
        return;
    }

    if channels == 2 && buffers.len() >= 2 {
        // Provide an optimized loop for stereo.
        let (buf0, buf1) = buffers.split_first_mut().unwrap();
        let buf0 = &mut buf0[buffer_range.clone()];
        let buf1 = &mut buf1[0][buffer_range.clone()];

        let src_slice = &data[start_sample * 2..(start_sample + samples) * 2];

        for (src_chunk, (buf0_s, buf1_s)) in src_slice
            .chunks_exact(2)
            .zip(buf0.iter_mut().zip(buf1.iter_mut()))
        {
            *buf0_s = convert(src_chunk[0]);
            *buf1_s = convert(src_chunk[1]);
        }

        return;
    }

    let src_slice = &data[start_sample * channels..(start_sample + samples) * channels];
    for (ch_i, buf_ch) in (0..channels).zip(buffers.iter_mut()) {
        for (src_chunk, buf_s) in src_slice
            .chunks_exact(channels)
            .zip(buf_ch[buffer_range.clone()].iter_mut())
        {
            *buf_s = convert(src_chunk[ch_i]);
        }
    }
}

/// A helper method to fill buffers from a resource of deinterleaved samples.
pub fn fill_buffers_deinterleaved<T: Clone + Copy, V: AsRef<[T]>>(
    buffers: &mut [&mut [f32]],
    buffer_range: Range<usize>,
    start_sample: usize,
    data: &[V],
    convert: impl Fn(T) -> f32,
) {
    let start_sample = start_sample as usize;
    let samples = buffer_range.end - buffer_range.start;

    if data.len() == 2 && buffers.len() >= 2 {
        // Provide an optimized loop for stereo.
        let (buf0, buf1) = buffers.split_first_mut().unwrap();
        let buf0 = &mut buf0[buffer_range.clone()];
        let buf1 = &mut buf1[0][buffer_range.clone()];
        let s0 = &data[0].as_ref()[start_sample..start_sample + samples];
        let s1 = &data[1].as_ref()[start_sample..start_sample + samples];

        for i in 0..samples {
            buf0[i] = convert(s0[i]);
            buf1[i] = convert(s1[i]);
        }

        return;
    }

    for (buf, ch) in buffers.iter_mut().zip(data.iter()) {
        for (buf_s, &ch_s) in buf[buffer_range.clone()]
            .iter_mut()
            .zip(ch.as_ref()[start_sample..start_sample + samples].iter())
        {
            *buf_s = convert(ch_s);
        }
    }
}

/// A helper method to fill buffers from a resource of deinterleaved `f32` samples.
pub fn fill_buffers_deinterleaved_f32<V: AsRef<[f32]>>(
    buffers: &mut [&mut [f32]],
    buffer_range: Range<usize>,
    start_sample: usize,
    data: &[V],
) {
    let start_sample = start_sample as usize;

    for (buf, ch) in buffers.iter_mut().zip(data.iter()) {
        buf[buffer_range.clone()].copy_from_slice(
            &ch.as_ref()[start_sample..start_sample + buffer_range.end - buffer_range.start],
        );
    }
}

#[cfg(feature = "symphonium")]
/// A wrapper around [`symphonium::DecodedAudio`] which implements the
/// [`SampleResource`] trait.
pub struct DecodedAudio(pub symphonium::DecodedAudio);

#[cfg(feature = "symphonium")]
impl DecodedAudio {
    pub fn duration_seconds(&self) -> f64 {
        self.0.frames() as f64 / self.0.sample_rate() as f64
    }
}

#[cfg(feature = "symphonium")]
impl SampleResource for DecodedAudio {
    fn num_channels(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.0.channels()).unwrap()
    }

    fn len_samples(&self) -> u64 {
        self.0.frames() as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        let channels = self.0.channels().min(buffers.len());

        if channels == 2 {
            let (b1, b2) = buffers.split_first_mut().unwrap();

            self.0.fill_stereo(
                start_sample as usize,
                &mut b1[buffer_range.clone()],
                &mut b2[0][buffer_range.clone()],
            );
        } else {
            for (ch_i, b) in buffers[0..channels].iter_mut().enumerate() {
                self.0
                    .fill_channel(ch_i, start_sample as usize, &mut b[buffer_range.clone()])
                    .unwrap();
            }
        }
    }
}

#[cfg(feature = "symphonium")]
impl From<symphonium::DecodedAudio> for DecodedAudio {
    fn from(data: symphonium::DecodedAudio) -> Self {
        Self(data)
    }
}

#[cfg(feature = "symphonium")]
/// A wrapper around [`symphonium::DecodedAudioF32`] which implements the
/// [`SampleResource`] trait.
pub struct DecodedAudioF32(pub symphonium::DecodedAudioF32);

#[cfg(feature = "symphonium")]
impl DecodedAudioF32 {
    pub fn duration_seconds(&self, sample_rate: u32) -> f64 {
        self.0.frames() as f64 / sample_rate as f64
    }
}

#[cfg(feature = "symphonium")]
impl SampleResource for DecodedAudioF32 {
    fn num_channels(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.0.channels()).unwrap()
    }

    fn len_samples(&self) -> u64 {
        self.0.frames() as u64
    }

    fn fill_buffers(
        &self,
        buffers: &mut [&mut [f32]],
        buffer_range: Range<usize>,
        start_sample: u64,
    ) {
        fill_buffers_deinterleaved_f32(buffers, buffer_range, start_sample as usize, &self.0.data);
    }
}

#[cfg(feature = "symphonium")]
impl From<symphonium::DecodedAudioF32> for DecodedAudioF32 {
    fn from(data: symphonium::DecodedAudioF32) -> Self {
        Self(data)
    }
}

/// A helper method to load an audio file from a path using Symphonium.
///
/// * `loader` - The symphonium loader.
/// * `path`` - The path to the audio file stored on disk.
/// * `sample_rate` - The sample rate of the audio stream.
/// * `resample_quality` - The quality of the resampler to use.
#[cfg(feature = "symphonium")]
pub fn load_audio_file<P: AsRef<std::path::Path>>(
    loader: &mut symphonium::SymphoniumLoader,
    path: P,
    #[cfg(feature = "resampler")] sample_rate: u32,
    #[cfg(feature = "resampler")] resample_quality: symphonium::ResampleQuality,
) -> Result<DecodedAudio, symphonium::error::LoadError> {
    loader
        .load(
            path,
            #[cfg(feature = "resampler")]
            Some(sample_rate),
            #[cfg(feature = "resampler")]
            resample_quality,
            None,
        )
        .map(|d| DecodedAudio(d))
}

/// A helper method to load an audio file from a custom source using Symphonium.
///
/// * `loader` - The symphonium loader.
/// * `source` - The audio source which implements the [`MediaSource`] trait.
/// * `hint` -  An optional hint to help the format registry guess what format reader is appropriate.
/// * `sample_rate` - The sample rate of the audio stream.
/// * `resample_quality` - The quality of the resampler to use.
#[cfg(feature = "symphonium")]
pub fn load_audio_file_from_source(
    loader: &mut symphonium::SymphoniumLoader,
    source: Box<dyn symphonium::symphonia::core::io::MediaSource>,
    hint: Option<symphonium::symphonia::core::probe::Hint>,
    #[cfg(feature = "resampler")] sample_rate: u32,
    #[cfg(feature = "resampler")] resample_quality: symphonium::ResampleQuality,
) -> Result<DecodedAudio, symphonium::error::LoadError> {
    loader
        .load_from_source(
            source,
            hint,
            #[cfg(feature = "resampler")]
            Some(sample_rate),
            #[cfg(feature = "resampler")]
            resample_quality,
            None,
        )
        .map(|d| DecodedAudio(d))
}

#[cfg(feature = "symphonium")]
/// A helper method to convert a [`symphonium::DecodedAudio`] resource into
/// a [`SampleResource`].
pub fn decoded_to_resource(data: symphonium::DecodedAudio) -> Arc<dyn SampleResource> {
    Arc::new(DecodedAudio(data))
}

#[cfg(feature = "symphonium")]
/// A helper method to convert a [`symphonium::DecodedAudioF32`] resource into
/// a [`SampleResource`].
pub fn decoded_f32_to_resource(data: symphonium::DecodedAudioF32) -> Arc<dyn SampleResource> {
    Arc::new(DecodedAudioF32(data))
}
