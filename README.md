# Smithay Toolkit + Glow examples

Most of the code is vibed.

```
cargo run --release --bin glow-with-glutin
cargo run --release --bin glow-with-wayland-egl
cargo run --release --bin wgpu-27
```

Memory usage results with AMD RX 9070 XT:

| Program | USS Memory | GPU Memory |
|---------|-----------|-----------|
| WGPU+Vulkan 27 | 26MB | 399MB |
| Glow with Glutin | 18MB | 88MB |
| Glow with Wayland-EGL | 18MB | 58MB |

Running with ./runall.sh, maximizing the windows to a 4k monitor.