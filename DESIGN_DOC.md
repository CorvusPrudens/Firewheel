# Firewheel Design Document

## Overview

Both the Rust ecosystem and the libre game engine ecosystem as a whole are in need of a powerful, flexible, and libre audio engine for games. Firewheel aims to provide developers with a powerful and modular solution for constructing custom audio experiences.

## Goals for First Release

* [x] Modular design that can be run on any backend that provides an audio stream.
    * [ ] (partially complete) [CPAL] backend. This gives us support for Windows, Mac, Linux, Android, iOS, and WebAssembly.
* [x] Flexible audio graph engine (supports any directed, acyclic graph with support for one-to-many connections)
* [x] Cycle detection for invalid audio graphs
* Key built-in nodes:
    * [x] volume (minimum value mutes)
    * [ ] stereo panning
    * [ ] stereo width
    * [x] sum
    * [x] hard clip
    * [x] mono to stereo
    * [x] stereo to mono
    * [ ] decibel (peak) meter
    * [x] beep test (generates a sine wav for testing)
    * [ ] sampler node (with support for looping audio)
    * [ ] simple spatial positioning (only the simplest implementation for first release)
* [ ] (api still a wip) Custom audio node API allowing for a plethora of 3rd party generators and effects
* [x] Silence optimizations (avoid processing if the audio buffer contains all zeros, useful when using "pools" of nodes where the majority of the time nodes are unused.)
* [ ] Basic tweening support for volume, pan, and spatial positioning nodes
* [ ] A general purpose "preset" graph with an easy-to-use interface
* [ ] Support for loading a wide variety of audio formats (using [Symphonium](https://github.com/MeadowlarkDAW/symphonium))
* [x] Fault tolerance for audio streams (The game shouldn't crash just because the player accidentally unplugged their headphones.)
* [x] Properly respect realtime constraints (no mutexes!)
* [x] Windows, Mac, and Linux support 
* [ ] WebAssembly support (Note special considerations must be made about the design of the threading model.)

## Later Goals

* Extra built-in nodes:
    * [ ] filters (lowpass, highpass, bandpass)
    * [ ] echo
    * [ ] delay compensation
    * [ ] convolutional reverb (user can load any own impulse response they want)
    * [ ] better spatial positioning with sound absorption capabilities
    * [ ] doppler stretching on sampler node
    * [ ] disk streaming on sampler node (using [creek](https://github.com/MeadowlarkDAW/creek))
    * [ ] networking streaming in WebAssembly
    * [ ] oscilloscope meter
* [ ] Basic [CLAP] plugin hosting (non-WebAssembly only)
* [ ] Animation curve support
* [ ] Snapping events to musical beats (useful for rhythm games)
* [ ] [RtAudio](https://github.com/thestk/rtaudio) backend
* [ ] [SDL](https://github.com/libsdl-org/SDL) backend
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
* `firewheel-graph` - Contains the core audio graph engine, along with some basic nodes such as volume and sum.
* `firewheel-sampler` - Contains the sampler node which plays audio files.
* `firewheel-spatial` - Contains the spatial audio node.
* `firewheel-extra` - Contains extra audio node effects like reverbs, echos, and filters.
* `firewheel-cpal` - Contains the default [CPAL] backend.
* (root crate) - Ties everything together and provides an optional general-purpose "graph preset" with an easy-to-use interface.

## Noteworthy Parts of the Tech Stack

* [Symphonium](https://github.com/MeadowlarkDAW/symphonium) - An easy-to-use wrapper around [Symphonia](https://github.com/pdeljanov/Symphonia), which is a Rust-native audio file decoder.
* [creek](https://github.com/MeadowlarkDAW/creek) - Provides realtime disk streaming for audio files.
* [rubato](https://crates.io/crates/rubato) - Asynchronous/synchronous resampling library written in native Rust. This will be useful for creating the "doppler shift" effect in the sampler node. Also used by Symphonium and creek (TODO) for resampling audio files to the stream's sample rate.
* [CPAL] - Native Rust crate providing an audio backend for Windows, MacOS, Linux, Android, and iOS.
* [RtAudio-rs](https://github.com/BillyDM/rtaudio-rs) - Rust bindings to the RtAudio backend.
* [rtrb](https://crates.io/crates/rtrb) - A realtime-safe SPSC ring buffer.
* [triple_buffer](https://crates.io/crates/triple_buffer) - A realtime-safe triple buffer.
* [thunderdome](https://crates.io/crates/thunderdome) - A fast generational arena.
* [downcast-rs](https://crates.io/crates/downcast-rs) - Allows audio nodes to be downcasted to the desired type without the use of `Any`.
* [Clack](https://github.com/prokopyl/clack) - Safe Rust bindings to the [CLAP] plugin API, along with hosting.

## Audio Node API

See crates/firewheel-core/node.rs

## Backend API

TODO

## Engine Lifecycle

1. A context with an audio graph is initialized. This context is currently "inactive", and the audio graph cannot be mutated.
2. The context is "activated" using an audio stream given to it by the backend. A realtime-safe message channel is created, along with a processor (executor) that is sent to the audio stream. Then the audio graph is "compiled" into a schedule and sent to the executor over the message channel. If compiling fails, then the context will be deactivated again and return an error.
3. "Active" state:
    * 3.1 - Users can add, remove, mutate, and connect nodes in this state.
    * 3.2 - The user periodically calls the `update` method on the context (i.e. once every frame). This method checks for any changes in the graph, and compiles a new schedule if a change is detected. If there was an error compiling the graph, then the update method will return an error and a new schedule will not be created.
4. The context can become deactivated in one of two ways:
    * The user requests to deactivate context. This is necessary, for example, when changing the audio io devices in the game's settings. Dropping the context will also automatically deactivate it first.
    * The audio stream is interrupted (i.e. the user unplugged the audio device). In this case, it is up to the developer/backend to decide how to respond (i.e. automatically try to activate again with a different device, or falling back to a "dummy" audio device).

## Clocks and Events

TODO

## Silence Optimization

TODO

## Sampler

TODO

## Spatial Positioning

TODO

## WebAssembly Considerations

Since WebAssembly is one of the targets, special considerations must be made to make the engine and audio nodes work with it. These include (but are not limited to):

TODO

And obviously hosting [CLAP] plugins is not possible in WebAssembly, so that feature will simply be disabled when compiling to that platform.


[CPAL]: https://github.com/RustAudio/cpal
[CLAP]: https://github.com/free-audio/clap