# Firewheel Design Document

## Overview

Both the Rust ecosystem and the libre game engine ecosystem as a whole are in need of a powerful, flexible, and libre audio engine for games. Firewheel aims to provide developers with a powerful and modular solution for constructing custom interactive audio experiences.

> #### Why the name "Firewheel"?
> The [firewheel](https://en.wikipedia.org/wiki/Gaillardia_pulchella) (aka "Indian Blanket", scientific name Gaillardia Pulchella) is a wildflower native to the Midwest USA. I just thought it was a cool looking flower with a cool name. :)

## Goals for First Release

* [x] Modular design that can be run on any backend that provides an audio stream.
    * [x] [CPAL] backend. This gives us support for Windows, Mac, Linux, Android, iOS, and WebAssembly.
* [x] Flexible audio graph engine (supports any directed, acyclic graph with support for both one-to-many and many-to-one connections)
* [x] Cycle detection for invalid audio graphs
* Key built-in nodes:
    * [x] volume (minimum value mutes)
    * [x] stereo panning
    * [x] stereo to mono
    * [x] decibel (peak) meter
    * [x] beep test (generates a sine wav for testing)
    * [x] stream writer (implemented but not tested yet) (put raw audio samples into the graph from another thread)
    * [x] stream reader (implemented but not tested yet) (read samples directly from the audio graph from another thread)
    * [x] sampler node
    * [ ] simple spatial positioning (only the simplest implementation for first release)
* [x] Custom audio node API allowing for a plethora of 3rd party generators and effects
* [x] Silence optimizations (avoid processing if the audio buffer contains all zeros, useful when using "pools" of nodes where the majority of the time nodes are unused.)
* [x] Support for loading a wide variety of audio formats (using [Symphonium](https://github.com/MeadowlarkDAW/symphonium))
* [x] Fault tolerance for audio streams (The game shouldn't crash just because the player accidentally unplugged their headphones.)
* [x] Option to hard clip outputs at 0dB to help protect the system's speakers.
* [x] Properly respect realtime constraints (no mutexes!)
* [x] Windows, Mac, and Linux support 
* [ ] Verify WebAssembly support (Note special considerations must be made about the design of the threading model.)

## Later Goals

* [ ] Sequencing support for the sampler node
* [ ] A `BlendingSamplerNode` that can seamlessly blend between multiple music tracks
* Extra built-in nodes:
    * [ ] delay compensation
    * [ ] convolution (user can load any impulse response they want to create effects like reverbs)
    * [ ] echo
    * [ ] filters (lowpass, highpass, bandpass)
* [ ] Doppler stretching (pitch shifting) on sampler node
* [ ] Basic [CLAP] plugin hosting (non-WebAssembly only)
* [ ] A `SampleResource` with disk streaming support (using [creek](https://github.com/MeadowlarkDAW/creek))
* [ ] A `SampleResource` with network streaming support
* [ ] Better spatial positioning with sound absorption capabilities
* [ ] [RtAudio](https://github.com/thestk/rtaudio) backend
* [ ] [Interflow](https://github.com/SolarLiner/interflow) backend
* [ ] C bindings

## Non-Goals

* MIDI on the audio-graph level (It will still be possible to create a custom sampler/synthesizer that reads a MIDI file as input.)
* Parameter events on the audio-graph level (as in you can't pass parameter events from one node to another)
* Connecting to system MIDI devices
* Built-in synthesizer instruments (This can still be done with third-party nodes/CLAP plugins.)
* Advanced mixing effects like parametric EQs, compressors, and limiters (This again can be done with third-party nodes/CLAP plugins.) Though a compressor and/or limiter might be added to the official library if it is deemed a common enough use case.
* GUIs for hosted CLAP plugins (This is a game audio engine, not a DAW audio engine.)
* Multi-threaded audio graph processing (This would make the engine a lot more complicated, and it is probably overkill for games.)
* VST, VST3, LV2, and AU plugin hosting

## Codebase Overview

* `firewheel-core` - Contains common types and utilities shared by Firewheel crates. It also houses the audio node API.
* `firewheel-graph` - Contains the core audio graph engine.
* `firewheel-nodes` - Contains the built-in factory nodes.
* `firewheel-cpal` - Contains the default [CPAL] backend.
* (root crate) - Ties everything together and provides an optional general-purpose "graph preset" with an easy-to-use interface.

## Noteworthy Parts of the Tech Stack

* [Symphonium](https://github.com/MeadowlarkDAW/symphonium) - An easy-to-use wrapper around [Symphonia](https://github.com/pdeljanov/Symphonia), which is a Rust-native audio file decoder.
* [creek](https://github.com/MeadowlarkDAW/creek) - Provides realtime disk streaming for audio files.
* [rubato](https://crates.io/crates/rubato) - Asynchronous/synchronous resampling library written in native Rust. This will be useful for creating the "doppler shift" effect in the sampler node. Also used by Symphonium and creek (TODO) for resampling audio files to the stream's sample rate.
* [CPAL] - Native Rust crate providing an audio backend for Windows, MacOS, Linux, Android, and iOS.
* [RtAudio-rs](https://github.com/BillyDM/rtaudio-rs) - Rust bindings to the RtAudio backend.
* [Interflow](https://github.com/SolarLiner/interflow) - A new experimental cross-platform audio backend. Might replace CPAL as the default backend in the future.
* [ringbuf](https://crates.io/crates/ringbuf) - A realtime-safe SPSC ring buffer
* [thunderdome](https://crates.io/crates/thunderdome) - A fast generational arena.
* [Clack](https://github.com/prokopyl/clack) - Safe Rust bindings to the [CLAP] plugin API, along with hosting.

## Audio Node API

See [crates/firewheel-core/src/node.rs](crates/firewheel-core/src/node.rs)

## Backend API

Audio backends should have the following features:

* Retrieve a list of audio output and/or audio input devices so games can let the user choose which audio devices to use in the game's setting GUI.
* Spawn an audio stream with the chosen input/output devices (or `None` which specifies to use the default device).
    * If the device is not found, try falling back to the default audio device first before returning an error (if the user specified that they want to fall back).
    * If no default device is found, try falling back to a "dummy" audio device first before returning an error (if the user specified that they want to fall back).
* While the stream is running, the internal clock should be updated accordingly before calling `FirewheelProcessor::process()`. (See the `Clocks and Events` section below.)
* If an error occurs, notify the user of the error when they call the `update()` method. From there the user can decide how to respond to the error (try to reconnect, fallback to a different device, etc.)

## Engine Lifecycle

1. A context with an audio graph is initialized.
2. The context is "activated" using an audio stream given to it by the backend. A realtime-safe message channel is created, along with a processor (executor) that is sent to the audio stream. Then the audio graph is "compiled" into a schedule and sent to the executor over the message channel. If compiling fails, then the context will be deactivated again and return an error.
3. "Active" state:
    - The user periodically calls the `update` method on the context (i.e. once every frame). This method first flushes any events that are in the queue and sends them to the audio thread. (Flushing events as a group like this ensures that events that are expected to happen on the same process cycle don't happen on different process cycles.) Then this method checks for any changes in the graph, and compiles a new schedule if a change is detected. If there was an error compiling the graph, then the update method will return an error and a new schedule will not be created.
4. The context can become deactivated in one of two ways:
    * a. The user requests to deactivate the context. This is necessary, for example, when changing the audio io devices in the game's settings. Dropping the context will also automatically deactivate it first.
    * b. The audio stream is interrupted (i.e. the user unplugged the audio device). In this case, it is up to the developer/backend to decide how to respond (i.e. automatically try to activate again with a different device, or falling back to a "dummy" audio device).

## Clocks and Events

There are three clocks in the audio stream: the seconds clock, the sample clock, and the musical clock.

### Seconds Clock

This clock is recommended for most general use cases. It counts the total number of seconds (as an `f64` value) that have elapsed since the start of the audio stream. This value is read from the OS's native audio API where possible, so it is quite accurate and it correctly accounts for any output underflows that may occur.

Usage of the clock works like this:

1. Before sending an event to an audio node, the user calls `AudioGraph::clock_now()` to retrieve the current clock time.
2. For any event type that accepts an `EventDelay` parameter, the user will schedule the event like so: `EventDelay::DelayUntilSeconds(AudioGraph::clock_now() + desired_amount_of_delay)`.

### Sample clock

The works the same as `Seconds Clock`, except it simply counts the total number of samples that have been processed since the stream was started. The is very accurate, but it does not correctly account for any output underflows that may occur.

### Musical Clock

This clock is manually started, paused, resumed, and stopped by the user. It counts the number of musical beats (as an `f64` value) that have elapsed since the `MusicalTransport` was started. This clock is ideal for syncing events to a musical tempo. Though like the sample clock, it does not account for any output underflows that may occur. Instead, the user is expected to poll the current time of the clock from the context to keep their game in sync.

## Silence Optimizations

It is common to have a "pool of audio nodes" at the ready to accept work from a certain maximum number of concurrent audio instances in the game engine. However, this means that the majority of the time, most of these nodes will be unused which would lead to a lot of unnecessary processing.

To get around this, every audio buffer in the graph is marked with a "silence flag". Audio nodes can read `ProcInfo::in_silence_mask` to quickly check which input buffers contain silence. If all input buffers are silent, then the audio node can choose to skip processing.

Audio nodes which output audio also must notify the graph on which output channels should/do contain silence. See `ProcessStatus` in [node.rs](crates/firewheel-core/src/node.rs) for more details.

## Sampler

The sampler nodes are used to play back audio files (sound FX, music, etc.). Samplers can play back any resource which implements the `SampleResource` trait in [sample_resource.rs](crates/firewheel-core/src/sample_resource.rs). Using a trait like this gives the game engine control over how to load and store audio assets, i.e. by using a crate like [Symphonium](https://github.com/MeadowlarkDAW/symphonium).

## Spatial Positioning

This node makes an audio stream appear as if it is "emanating" from a point in 3d space.

For the first release this node will have three parameters:

* A 3D vector which describes the distance and direction of the sound source from the listener.
* A "damping factor", which describes how much to dampen the volume of the sound based on the distance of the source.
* An "attenuation factor", which describes how much to attenuate the high frequencies of a sound based on the distance of the source.

This node can also accept anything that implements the `AnimationCurve` (TODO) trait to apply an animation curve.

In later releases, game engines can use additional "wall absorption" parameters to more realistically make sounds appear as if they are playing on the other side of a wall. (Note raycasting will not be part of this node, the game engine must do the raycasting itself).

We can also possibly look into more sophisticated stereo surround sound techniques if given the resources and talent.

## Other Key Nodes

### VolumeNode

This node simply changes the volume. It can also accept anything that implements the `AnimationCurve` (TODO) trait to apply an animation curve.

### PanNode

This node pans a stereo stream left/right. It can also accept anything that implements the `AnimationCurve` (TODO) trait to apply an animation curve.

### MixNode

This node sums streams together into a single stream.

### StereoToMonoNode

This node turns a stereo stream into a mono stream.

### TripleBufferOutNode

This node stores the latest samples in the audio stream into a triple buffer, allowing the game engine to read the raw samples. This can be useful for creating visual effects like oscilloscopes and spectrometers.

### TripleBufferInNode

This node allows the game engine to insert samples into the audio graph from any thread. This can be useful, for example, playing back voice chat from over the network.

### DelayCompNode

This node simply delays the stream by a certain sample amount. Useful for preventing phasing issues between parallel streams.

### DecibelMeterNode

This nodes measures the peak volume of a stream in decibels. This can be used to create meters in the GUI or triggering game events based on peak volume.

### ConvolutionNode

This node accepts anything that implements the `ImpulseResponse` trait (TODO) to create effects like reverbs. Similar to the `LoopingSamplerNode`, the user can blend seamlessly between multiple impulse responses (useful for creating effects like smoothly changing the reverb when the player enters a different room).

### EchoNode

This node produces an "echo" effect on an audio stream.

### FilterNode

Provides basic filtering effects like lowpass, highpass, and bandpass. It can also accept anything that implements the `AnimationCurve` (TODO) trait to apply an animation curve.

These filters should use the Simper SVF model as described in https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf due to its superior quality when being modulated. (Coefficient equations are given at the bottom of the paper).

## WebAssembly Considerations

Since WebAssembly (WASM) is one of the targets, special considerations must be made to make the engine and audio nodes work with it. These include (but are not limited to):

* No C or System Library Dependencies
    * Because of this, hosting [CLAP] plugins is not possible in WASM. So that feature will be disabled when compiling to that platform.
* No File I/O
    * Asset loading is out of scope of this project. Game engines themselves should be in charge of loading assets.
* Don't Spawn Threads
    * The audio backend (i.e. [CPAL]) should be in charge of spawning the audio thread.
    * While the [creek](https://github.com/MeadowlarkDAW/creek) crate requires threads, file operations aren't supported in WASM anyway, so this crate can just be disabled when compiling to WASM.
* Don't Block Threads

[CPAL]: https://github.com/RustAudio/cpal
[CLAP]: https://github.com/free-audio/clap