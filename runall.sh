#!/bin/bash

echo "Running all tests:"
cargo run --release --bin glow-with-glutin &
cargo run --release --bin glow-with-wayland-egl &
cargo run --release --bin wgpu-27 &
wait
