<div align="center"><img src="./assets/logo-512.png" width="64px" height="64px"/><h1>Firewheel</h1></div>

[![Documentation](https://docs.rs/firewheel/badge.svg)](https://docs.rs/firewheel)
[![Crates.io](https://img.shields.io/crates/v/firewheel.svg)](https://crates.io/crates/firewheel)
[![License](https://img.shields.io/crates/l/firewheel.svg)](https://github.com/BillyDM/firewheel/blob/main/LICENSE-APACHE)

*Work In Progress, not ready for any kind of use*

Firewheel aims to be a fully-featured libre open source audio graph engine for games and other applications!

## Key Planned Goals

* Modular design that can be run on any backend that provides an audio stream
    * Default backend supporting Windows, Mac, Linux, Android, iOS, and WebAssembly
* Flexible audio graph engine (supports any directed, acyclic graph with support for both one-to-many and many-to-one connections)
* A suite of essential built-in audio nodes:
    * gain, stereo panning, and summing
    * versatile sampler
    * spatial positioning (make a sound "emanate" from a point in 3d space)
    * triple buffering for inserting/reading raw samples from any thread
* Custom audio node API allowing for a plethora of 3rd party generators and effects
* Basic [CLAP](https://cleveraudio.org/) plugin hosting (non-WASM only), allowing for more open source and proprietary 3rd party effects and synths
* Silence optimizations (avoid processing if the audio buffer contains all zeros, useful when using "pools" of nodes where the majority of the time nodes are unused)
* Ability to add "sequences" to certain nodes (i.e. automation and sequences of events).
* A general purpose "preset" graph with an easy-to-use interface
* Support for loading a wide variety of audio formats
* Fault tolerance for audio streams (The game shouldn't stop or crash just because the player accidentally unplugged their headphones.)
* Properly respect realtime constraints (no mutexes!)
* C bindings

> Not all of the above features are planned for the first release. See the [Design Document] for more details.

## Motivation

While Firewheel is its own standalone project, we are also working closely with the [Bevy](https://bevyengine.org/) game engine to make it Bevy's default audio engine.

## Get Involved

Join the discussion in the [Firewheel Discord Server](https://discord.gg/m42dPpRm) or in the [Bevy Discord Server](https://discord.gg/bevy) under the `working-groups -> Better Audio` channel!

If you are interested in contributing code, first read the [Design Document] and then visit the [Project Board]() (TODO).

If you are a game developer that wishes to see this project flourish, please consider donating or sponsoring! Links are on the right side of the GitHub page. 🌼

## License

Licensed under either of

* Apache License, Version 2.0, (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0), or
* MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT)

at your option.

[Design Document]: DESIGN_DOC.md