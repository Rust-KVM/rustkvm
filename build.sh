#!/bin/bash
set -e

# Function to check if a command exists
command_exists() {
  command -v "$1" >/dev/null 2>&1
}

# Check for mise and install if not present
if ! command_exists mise; then
  echo "mise is not installed. Attempting to install..."
  if command_exists curl; then
    curl https://mise.run | sh
    # Add mise to PATH for the current script execution
    export PATH="$HOME/.local/bin:$PATH"
    if ! command_exists mise; then
      echo "Mise installation appears to have failed." >&2
      echo "Please install mise manually (https://mise.run) and try again." >&2
      exit 1
    fi
    echo "mise has been installed successfully."
  else
    echo "curl is not installed. Cannot install mise automatically." >&2
    echo "Please install curl, or install mise manually (https://mise.run) and try again." >&2
    exit 1
  fi
else
    echo "mise is already installed."
fi

# Check for pnpm
if ! command_exists pnpm; then
  echo "pnpm is not installed. Installing with mise..."
fi

mise use pnpm@latest

# Check for Rust (rustup and cargo)
if ! command_exists rustc; then
  echo "Rust is not installed. Installing with mise..."
fi

mise use rust@latest

echo "All necessary toolchains are installed."

# build frontend
echo "Building frontend..."
(cd frontend && pnpm install && pnpm build)
echo "Frontend build complete."

# build backend
echo "Building backend..."
cargo build --release
echo "Backend build complete."

echo "Project build finished successfully."
