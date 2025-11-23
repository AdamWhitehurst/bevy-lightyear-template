#!/bin/bash
set -e

echo "Setting up Lightyear development environment..."

# Install WASM target
echo "Installing wasm32-unknown-unknown target..."
rustup target add wasm32-unknown-unknown

# Install Bevy CLI
echo "Installing Bevy CLI..."
cargo install bevy_cli

# Generate certificates
echo "Generating certificates..."
sh certificates/generate.sh

echo "Setup complete!"
