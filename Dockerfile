# Stage 1: Builder
FROM ubuntu:24.04 as builder

# Install build dependencies.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    curl \
    git \
    pkg-config \
    libssl-dev \
    patchelf \
    python3 \
    python3-venv \
    python3-pip \
 && rm -rf /var/lib/apt/lists/*

# Install Rust via rustup.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install uv (the Astral uv package manager).
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:${PATH}"

WORKDIR /app
# Copy all project files.
COPY . /app

# Install desired CPython versions in the uv cache (if needed).
RUN uv python install 3.12 3.13

# Create a virtual environment using uv (this creates /app/.venv).
RUN uv venv

# Activate the virtual environment.
ENV PATH="/app/.venv/bin:${PATH}"

# Install maturin (via uv pip) in the virtual environment.
RUN uv pip install maturin

# Set PYO3 compatibility flag.
ENV PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1

# Build and install the Python extension into the virtual environment.
RUN uv run maturin develop --release --features extension-module

# Build the Rust CLI executable.
RUN cargo build --release

# Copy the CLI executable to /usr/local/bin.
RUN cp target/release/s3Rust-cli /usr/local/bin/s3Rust-cli

# (Optional cleanup in builder stage could go here.)

# Stage 2: Runtime (Runner)
FROM ubuntu:24.04 as runtime

# Install only runtime dependencies.
RUN apt-get update && apt-get install -y --no-install-recommends \
    patchelf  python3  python3-venv  python3-pip \
    vim less net-tools iputils-ping nano \
 && rm -rf /var/lib/apt/lists/*

# Copy the built CLI executable.
COPY --from=builder /usr/local/bin/s3Rust-cli /usr/local/bin/s3Rust-cli

# Copy the virtual environment with the installed Python extension.
COPY --from=builder /app/.venv /app/.venv

# Copy the entire project directory so local Python programs are available.
COPY --from=builder /app /app

# Copy the uv installation from builder.
COPY --from=builder /root/.local /root/.local

WORKDIR /app
# Make sure uv (and the venv) is on the PATH.
ENV PATH="/root/.local/bin:/app/.venv/bin:${PATH}"

# Cleanup build artifacts
RUN rm -rf  /app/target/release/build /app/target/release/deps /app/src
RUN rm -rf  /app/Cargo.* /app/*.sh /app/Dockerfile* /app/*.toml


# Default to bash for interactive testing.
CMD ["/bin/bash"]

