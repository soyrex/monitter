#!/usr/bin/env python3
"""Start the pinned GPU-only MLX server with native-compatible strict input limits."""
import sys
from strict_sequence import install
from mlx_omarchy_laya import sequence
install(sequence)
from mlx_omarchy_laya.server import serve_main

if __name__ == '__main__':
    serve_main(sys.argv[1:])
