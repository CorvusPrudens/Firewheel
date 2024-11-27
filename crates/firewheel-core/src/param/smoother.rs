use std::fmt;
use std::ops;
use std::slice;

use crate::dsp::smoothing_filter;

/// The configuration for a [`ParamSmoother`]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmootherConfig {
    /// The amount of smoothing in seconds
    ///
    /// By default this is set to 5 milliseconds.
    pub smooth_secs: f32,
    /// The threshold at which the smoothing will complete
    ///
    /// By default this is set to `0.00001`.
    pub settle_epsilon: f32,
}

impl Default for SmootherConfig {
    fn default() -> Self {
        Self {
            smooth_secs: smoothing_filter::DEFAULT_SMOOTH_SECONDS,
            settle_epsilon: smoothing_filter::DEFAULT_SETTLE_EPSILON,
        }
    }
}

/// The status of a [`ParamSmoother`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmootherStatus {
    /// Not currently smoothing. All values in [`ParamSmoother::output`]
    /// will contain the same value.
    Inactive,
    /// Currently smoothing but will become deactivated on the next process
    /// cycle. Values in [`ParamSmoother::output`] will NOT be all the same.
    Deactivating,
    /// Currently smoothing. Values in [`ParamSmoother::output`] will NOT
    /// be all the same.
    Active,
}

impl SmootherStatus {
    fn is_active(&self) -> bool {
        self != &SmootherStatus::Inactive
    }
}

/// The output of a [`ParamSmoother`]
pub struct SmoothedOutput<'a> {
    pub values: &'a [f32],
    pub status: SmootherStatus,
}

impl<'a> SmoothedOutput<'a> {
    pub fn is_smoothing(&self) -> bool {
        self.status.is_active()
    }
}

impl<'a, I> ops::Index<I> for SmoothedOutput<'a>
where
    I: slice::SliceIndex<[f32]>,
{
    type Output = I::Output;

    #[inline(always)]
    fn index(&self, idx: I) -> &I::Output {
        &self.values[idx]
    }
}

/// An automatically smoothed buffer of values for a parameter.
#[derive(Clone)]
pub struct ParamSmoother {
    output: Vec<f32>,
    target: f32,

    status: SmootherStatus,

    filter_coeff: smoothing_filter::Coeff,
    filter_state: f32,

    settle_epsilon: f32,
}

impl ParamSmoother {
    /// Create a new parameter smoothing filter.
    ///
    /// * `val` - The initial starting value
    /// * `sample_rate` - The sampling rate
    /// * `max_block_samples` - The maximum number of samples that can
    /// appear in a processing block.
    /// * `config` - Additional options for a [`ParamSmoother`]
    pub fn new(val: f32, sample_rate: u32, max_block_samples: u32, config: SmootherConfig) -> Self {
        Self {
            status: SmootherStatus::Inactive,
            target: val,
            output: vec![val; max_block_samples as usize],
            filter_coeff: smoothing_filter::Coeff::new(sample_rate, config.smooth_secs),
            filter_state: val,
            settle_epsilon: config.settle_epsilon,
        }
    }

    /// Reset the filter with the new given initial value.
    pub fn reset(&mut self, val: f32) {
        if self.is_active() {
            self.status = SmootherStatus::Inactive;
            self.output.fill(val);
        } else if self.target != val {
            self.output.fill(val);
        }

        self.target = val;
        self.filter_state = val;
    }

    /// Set the new target value. If the value is different from the previous process
    /// cycle, then smoothing will begin.
    pub fn set(&mut self, val: f32) {
        if self.target == val {
            return;
        }

        self.target = val;
        self.status = SmootherStatus::Active;
    }

    /// Set the new target value.
    ///
    /// If `no_smoothing` is `false` and the value is different from the previous
    /// process cycle, then smoothing will begin.
    ///
    /// If `no_smoothing` is `true`, then the filter will be reset with the new
    /// value.
    pub fn set_with_smoothing(&mut self, val: f32, smoothing: bool) {
        if smoothing {
            self.set(val);
        } else {
            self.reset(val);
        }
    }

    /// The current target value that is being smoothed to.
    pub fn target_value(&self) -> f32 {
        self.target
    }

    /// Get the current value of the smoother, along with its status.
    ///
    /// Note, this will NOT update the filter. This only returns the most
    /// recently-processed sample.
    pub fn current_value(&self) -> (f32, SmootherStatus) {
        (self.filter_state, self.status)
    }

    /// Process the filter and return the smoothed output.
    ///
    /// If the filter is not currently smoothing, then no processing will occur and
    /// the output (which will contain all the same value) will simply be returned.
    pub fn process(&mut self, samples: usize) -> SmoothedOutput {
        let samples = samples.min(self.output.len());

        match self.status {
            SmootherStatus::Deactivating => {
                self.reset(self.target);
            }
            SmootherStatus::Active => {
                self.filter_state = smoothing_filter::process_into_buffer(
                    &mut self.output[..samples],
                    self.filter_state,
                    self.target,
                    self.filter_coeff,
                );

                if smoothing_filter::has_settled(
                    self.filter_state,
                    self.target,
                    self.settle_epsilon,
                ) {
                    self.status = SmootherStatus::Deactivating;
                }
            }
            _ => {}
        }

        SmoothedOutput {
            values: &self.output[..samples],
            status: self.status,
        }
    }

    /// Set the new target value, process the filter, and return the smoothed output.
    /// If the value is different from the previous process cycle, then smoothing will
    /// begin.
    ///
    /// If the filter is not currently smoothing, then no processing will occur and
    /// the output (which will contain all the same value) will simply be returned.
    pub fn set_and_process(&mut self, val: f32, samples: usize) -> SmoothedOutput {
        self.set(val);
        self.process(samples)
    }

    /// Whether or not the filter is currently smoothing (`true`) or not (`false`)
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }

    /// Returns the current value if the filter is not currently smoothing, returns
    /// `None` otherwise.
    pub fn constant_value(&self) -> Option<f32> {
        if self.status.is_active() {
            None
        } else {
            Some(self.target)
        }
    }

    /// The maximum number of samples tha can appear in a single processing block.
    pub fn max_block_samples(&self) -> usize {
        self.output.len()
    }
}

impl fmt::Debug for ParamSmoother {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(concat!("ParamSmoother"))
            .field("output[0]", &self.output[0])
            .field("max_block_samples", &self.max_block_samples())
            .field("target", &self.target)
            .field("status", &self.status)
            .field("filter_state", &self.filter_state)
            .field("filter_coeff", &self.filter_coeff)
            .field("settle_epsilon", &self.settle_epsilon)
            .finish()
    }
}
