use std::num::NonZeroU32;

use firewheel_core::{
    channel_config::NonZeroChannelCount,
    clock::EventDelay,
    diff::{Diff, PathBuilder},
    node::NodeID,
};
use firewheel_cpal::FirewheelContext;
use firewheel_nodes::sampler::{PlaybackState, SamplerConfig, SamplerHandle, SamplerParams};
use smallvec::SmallVec;
use thunderdome::Arena;

pub trait FxChain: Default {
    /// Construct the nodes in the FX chain and connect them, returning a list of the
    /// new node ids.
    ///
    /// * `sampler_node_id` - The ID of the sampler node in this worker instance.
    /// * `sampler_num_channels` - The number of channels in the sampler node.
    /// * `dst_node_id` - The ID of the node that the last node in this FX chain should
    /// connect to.
    /// * `dst_num_channels` - The number of input channels on `dst_node_id`.
    /// * `cx` - The firewheel context.
    fn construct_and_connect(
        &mut self,
        sampler_node_id: NodeID,
        sampler_num_channels: NonZeroChannelCount,
        dst_node_id: NodeID,
        dst_num_channels: NonZeroChannelCount,
        cx: &mut FirewheelContext,
    ) -> Vec<NodeID>;
}

struct Worker<FX: FxChain> {
    sampler_params: SamplerParams,
    sampler_handle: SamplerHandle,
    sampler_id: NodeID,

    fx_state: FxChainState<FX>,

    assigned_worker_id: Option<WorkerID>,
}

pub struct FxChainState<FX: FxChain> {
    pub fx_chain: FX,
    pub node_ids: Vec<NodeID>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerID(thunderdome::Index);

impl WorkerID {
    pub const DANGLING: Self = Self(thunderdome::Index::DANGLING);
}

impl Default for WorkerID {
    fn default() -> Self {
        Self::DANGLING
    }
}

/// A pool of sampler nodes that can dynamically be assigned work.
///
/// Each worker also contains its own chain of effect nodes.
pub struct SamplerPool<FX: FxChain> {
    workers: Vec<Worker<FX>>,
    worker_ids: Arena<usize>,
}

impl<FX: FxChain> SamplerPool<FX> {
    /// Construct a new sampler pool.
    ///
    /// * `num_workers` - The total number of workers that can work in parallel. More workers
    /// will allow more samples to be played concurrently, but will also increase processing
    /// overhead. A value of `16` is a good place to start.
    /// * `config` - The configuration of the sampler nodes.
    /// * `dst_node_id` - The ID of the node that the last effect in each fx chain instance
    /// will connect to.
    /// * `dst_num_channels` - The number of input channels in `dst_node_id`.
    /// * `cx` - The firewheel context.
    pub fn new(
        num_workers: usize,
        config: SamplerConfig,
        dst_node_id: NodeID,
        dst_num_channels: NonZeroChannelCount,
        cx: &mut FirewheelContext,
    ) -> Self {
        assert_ne!(num_workers, 0);

        Self {
            workers: (0..num_workers)
                .map(|_| {
                    let sampler_params = SamplerParams::default();
                    let sampler_handle = SamplerHandle::new();

                    let sampler_id = cx.add_node(
                        sampler_handle.constructor(sampler_params.clone(), config.clone()),
                    );

                    let mut fx_chain = FX::default();

                    let fx_ids = fx_chain.construct_and_connect(
                        sampler_id,
                        config.channels,
                        dst_node_id,
                        dst_num_channels,
                        cx,
                    );

                    Worker {
                        sampler_params,
                        sampler_handle,
                        sampler_id,

                        fx_state: FxChainState {
                            fx_chain,
                            node_ids: fx_ids,
                        },

                        assigned_worker_id: None,
                    }
                })
                .collect(),
            worker_ids: Arena::with_capacity(num_workers),
        }
    }

    pub fn num_workers(&self) -> usize {
        self.workers.len()
    }

    pub fn play(
        &mut self,
        sampler_params: SamplerParams,
        delay: Option<EventDelay>,
        cx: &mut FirewheelContext,
        fx_chain: impl FnOnce(&mut FxChainState<FX>, &mut FirewheelContext),
    ) -> PlayResult {
        let mut idx = 0;
        let mut max_score = 0;
        for (i, worker) in self.workers.iter().enumerate() {
            if worker.assigned_worker_id.is_none() {
                idx = i;
                break;
            }

            let score = worker.sampler_handle.worker_score(&worker.sampler_params);

            if score == u64::MAX {
                idx = i;
                break;
            }

            if score > max_score {
                max_score = score;
                idx = i;
            }
        }

        let worker_id = WorkerID(self.worker_ids.insert(idx));

        let worker = &mut self.workers[idx];

        let old_worker_id = worker.assigned_worker_id.take();
        let was_playing_sequence = if let Some(old_worker_id) = old_worker_id {
            self.worker_ids.remove(old_worker_id.0);

            worker.sampler_handle.playback_state().is_playing()
        } else {
            false
        };

        worker.assigned_worker_id = Some(worker_id);
        worker.sampler_params = sampler_params;

        if let Some(delay) = delay {
            cx.queue_event_for(
                worker.sampler_id,
                worker
                    .sampler_handle
                    .sync_params_event(worker.sampler_params.clone(), false),
            );

            cx.queue_event_for(
                worker.sampler_id,
                worker
                    .sampler_handle
                    .start_or_restart_event(&worker.sampler_params, Some(delay)),
            );
        } else {
            cx.queue_event_for(
                worker.sampler_id,
                worker
                    .sampler_handle
                    .sync_params_event(worker.sampler_params.clone(), true),
            );
        }

        (fx_chain)(&mut worker.fx_state, cx);

        PlayResult {
            worker_id,
            old_worker_id,
            was_playing_sequence,
        }
    }

    pub fn sampler_params(&self, worker_id: WorkerID) -> Option<&SamplerParams> {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            Some(&self.workers[idx].sampler_params)
        } else {
            None
        }
    }

    pub fn playback_state(&self, worker_id: WorkerID) -> PlaybackState {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            self.workers[idx].sampler_handle.playback_state()
        } else {
            PlaybackState::Stopped
        }
    }

    pub fn fx_chain(&self, worker_id: WorkerID) -> Option<&FxChainState<FX>> {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            Some(&self.workers[idx].fx_state)
        } else {
            None
        }
    }

    pub fn fx_chain_mut(&mut self, worker_id: WorkerID) -> Option<&mut FxChainState<FX>> {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            Some(&mut self.workers[idx].fx_state)
        } else {
            None
        }
    }

    /// Pause the given worker.
    ///
    /// Returns `true` if a worker with the given ID exists, `false` otherwise.
    pub fn pause(&mut self, worker_id: WorkerID, cx: &mut FirewheelContext) -> bool {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            let worker = &mut self.workers[idx];

            cx.queue_event_for(worker.sampler_id, worker.sampler_handle.pause_event());

            true
        } else {
            false
        }
    }

    /// Resume the given worker.
    ///
    /// Returns `true` if a worker with the given ID exists, `false` otherwise.
    pub fn resume(&mut self, worker_id: WorkerID, cx: &mut FirewheelContext) -> bool {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            let worker = &mut self.workers[idx];

            cx.queue_event_for(
                worker.sampler_id,
                worker.sampler_handle.resume_event(&worker.sampler_params),
            );

            true
        } else {
            false
        }
    }

    /// Stop the given worker.
    ///
    /// This will invalidate the given `worker_id`.
    ///
    /// Returns `true` if a worker with the given ID exists and was stopped.
    pub fn stop(&mut self, worker_id: WorkerID, cx: &mut FirewheelContext) -> bool {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            self.worker_ids.remove(worker_id.0);

            let worker = &mut self.workers[idx];

            worker.assigned_worker_id = None;

            cx.queue_event_for(worker.sampler_id, worker.sampler_handle.stop_event());

            true
        } else {
            false
        }
    }

    /// Pause all workers.
    pub fn pause_all(&mut self, cx: &mut FirewheelContext) {
        for worker in self.workers.iter_mut() {
            if worker.assigned_worker_id.is_some() {
                cx.queue_event_for(worker.sampler_id, worker.sampler_handle.pause_event());
            }
        }
    }

    /// Resume all workers.
    pub fn resume_all(&mut self, cx: &mut FirewheelContext) {
        for worker in self.workers.iter_mut() {
            if worker.assigned_worker_id.is_some() {
                cx.queue_event_for(
                    worker.sampler_id,
                    worker.sampler_handle.resume_event(&worker.sampler_params),
                );
            }
        }
    }

    /// Stop all workers.
    pub fn stop_all(&mut self, cx: &mut FirewheelContext) {
        for worker in self.workers.iter_mut() {
            if let Some(_) = worker.assigned_worker_id.take() {
                cx.queue_event_for(worker.sampler_id, worker.sampler_handle.stop_event());
            }
        }

        self.worker_ids.clear();
    }

    /// Set the playhead for the given worker in seconds.
    ///
    /// Returns `true` if a worker with the given ID exists, `false` otherwise.
    pub fn set_playead_seconds(
        &mut self,
        worker_id: WorkerID,
        playhead_seconds: f64,
        cx: &mut FirewheelContext,
    ) -> bool {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            let worker = &mut self.workers[idx];

            cx.queue_event_for(
                worker.sampler_id,
                worker.sampler_handle.set_playhead_event(playhead_seconds),
            );

            true
        } else {
            false
        }
    }

    /// Set the playhead for the given worker in units of samples (of a single channel of audio).
    ///
    /// Returns `true` if a worker with the given ID exists, `false` otherwise.
    pub fn set_playead_samples(
        &mut self,
        worker_id: WorkerID,
        playhead_samples: u64,
        cx: &mut FirewheelContext,
    ) -> bool {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            let worker = &mut self.workers[idx];

            cx.queue_event_for(
                worker.sampler_id,
                worker
                    .sampler_handle
                    .set_playhead_samples_event(playhead_samples),
            );

            true
        } else {
            false
        }
    }

    /// Get the current playhead for the given worker in units of seconds.
    ///
    /// Returns `none` if a worker with the given ID does not exist.
    pub fn playhead_seconds(
        &mut self,
        worker_id: WorkerID,
        sample_rate: NonZeroU32,
    ) -> Option<f64> {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            let worker = &self.workers[idx];

            worker
                .sampler_handle
                .playhead_seconds(&worker.sampler_params, sample_rate)
        } else {
            None
        }
    }

    /// Get the current playhead for the given worker in units of samples (of a
    /// single channel of audio).
    ///
    /// Returns `none` if a worker with the given ID does not exist.
    pub fn playhead_samples(&mut self, worker_id: WorkerID) -> Option<u64> {
        if let Some(idx) = self.worker_ids.get(worker_id.0).copied() {
            let worker = &self.workers[idx];

            worker
                .sampler_handle
                .playhead_samples(&worker.sampler_params)
        } else {
            None
        }
    }

    /// Poll for the current number of active workers, and return a list of
    /// workers which have finished playing.
    ///
    /// Calling this method is optional.
    pub fn poll(&mut self) -> PollResult {
        let mut num_active_workers = 0;
        let mut finished_workers = SmallVec::new();

        for worker in self.workers.iter_mut() {
            if worker.assigned_worker_id.is_some() {
                if worker.sampler_handle.playback_state() == PlaybackState::Stopped {
                    finished_workers.push(worker.assigned_worker_id.take().unwrap());
                } else {
                    num_active_workers += 1;
                }
            }
        }

        PollResult {
            num_active_workers,
            finished_workers,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PollResult {
    /// The number of workers currently active.
    pub num_active_workers: usize,
    /// The worker IDs which have finished playing. These IDs are now
    /// invalidated.
    pub finished_workers: SmallVec<[WorkerID; 4]>,
}

/// The result of calling [`SamplerPool::play`].
pub struct PlayResult {
    /// The new ID of the worker assigned to play this sequence.
    pub worker_id: WorkerID,

    /// The ID that was previously assigned to this worker.
    pub old_worker_id: Option<WorkerID>,

    /// If this is `true`, then this worker was already playing a sequence, and that
    /// sequence has been stopped.
    pub was_playing_sequence: bool,
}

/// A default [`SamplerPool`] [`FxChain`] for 2D game audio.
///
/// This chain contains a single `VolumePan` node.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct VolumePanChain {
    pub volume_pan: firewheel_nodes::volume_pan::VolumePanParams,
    pub config: firewheel_nodes::volume_pan::VolumeNodeConfig,
}

impl VolumePanChain {
    pub fn set_params(
        &mut self,
        params: firewheel_nodes::volume_pan::VolumePanParams,
        node_ids: &[NodeID],
        cx: &mut FirewheelContext,
    ) {
        let node_id = node_ids[0];

        self.volume_pan.diff(
            &params,
            PathBuilder::default(),
            &mut cx.event_queue(node_id),
        );
    }
}

impl FxChain for VolumePanChain {
    fn construct_and_connect(
        &mut self,
        sampler_node_id: NodeID,
        sampler_num_channels: NonZeroChannelCount,
        dst_node_id: NodeID,
        dst_num_channels: NonZeroChannelCount,
        cx: &mut FirewheelContext,
    ) -> Vec<NodeID> {
        let volume_pan_params = firewheel_nodes::volume_pan::VolumePanParams::default();

        let volume_pan_node_id = cx.add_node(volume_pan_params.constructor(self.config));

        cx.connect(
            sampler_node_id,
            volume_pan_node_id,
            if sampler_num_channels.get().get() == 1 {
                &[(0, 0), (0, 1)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )
        .unwrap();

        cx.connect(
            volume_pan_node_id,
            dst_node_id,
            if dst_num_channels.get().get() == 1 {
                &[(0, 0), (1, 0)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )
        .unwrap();

        vec![volume_pan_node_id]
    }
}

/// A default [`SamplerPool`] [`FxChain`] for 3D game audio.
///
/// This chain contains a single `SpatialBasic` node.
#[cfg(feature = "spatial_basic_node")]
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct SpatialBasicChain {
    pub spatial_basic: firewheel_nodes::spatial_basic::SpatialBasicParams,
    pub config: firewheel_nodes::spatial_basic::SpatialBasicConfig,
}

#[cfg(feature = "spatial_basic_node")]
impl SpatialBasicChain {
    pub fn set_params(
        &mut self,
        params: firewheel_nodes::spatial_basic::SpatialBasicParams,
        node_ids: &[NodeID],
        cx: &mut FirewheelContext,
    ) {
        let node_id = node_ids[0];

        self.spatial_basic.diff(
            &params,
            PathBuilder::default(),
            &mut cx.event_queue(node_id),
        );
    }
}

#[cfg(feature = "spatial_basic_node")]
impl FxChain for SpatialBasicChain {
    fn construct_and_connect(
        &mut self,
        sampler_node_id: NodeID,
        sampler_num_channels: NonZeroChannelCount,
        dst_node_id: NodeID,
        dst_num_channels: NonZeroChannelCount,
        cx: &mut FirewheelContext,
    ) -> Vec<NodeID> {
        let spatial_basic_params = firewheel_nodes::spatial_basic::SpatialBasicParams::default();

        let spatial_basic_node_id = cx.add_node(spatial_basic_params.constructor(self.config));

        cx.connect(
            sampler_node_id,
            spatial_basic_node_id,
            if sampler_num_channels.get().get() == 1 {
                &[(0, 0), (0, 1)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )
        .unwrap();

        cx.connect(
            spatial_basic_node_id,
            dst_node_id,
            if dst_num_channels.get().get() == 1 {
                &[(0, 0), (1, 0)]
            } else {
                &[(0, 0), (1, 1)]
            },
            false,
        )
        .unwrap();

        vec![spatial_basic_node_id]
    }
}
